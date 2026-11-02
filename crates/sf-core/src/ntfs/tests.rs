//! Hand-built NTFS volume covering the cases that are hard to get out of a
//! real formatter on demand: a fragmented MFT, torn records, reused parent
//! records, reused clusters, hostile names. The real-NTFS fixture test in
//! `tests/ntfs_fixture.rs` covers what Windows actually writes.

use super::*;
use crate::block_device::FakeBlockDevice;
use crate::reader::{RangeSet, SafeReader};

const CS: usize = 4096;
const RS: usize = 1024;
const PART: usize = 1024 * 1024;
const CLUSTERS: usize = 1024;
const MFT_RUNS: [(u64, u64); 2] = [(8, 4), (8, 100)]; // (len, lcn): fragmented

fn fref(n: u64, seq: u16) -> u64 {
    n | (seq as u64) << 48
}

fn pad8(v: &mut Vec<u8>) {
    while v.len() % 8 != 0 {
        v.push(0);
    }
}

fn resident(typ: u32, id: u16, flags: u16, name: Option<&str>, value: &[u8]) -> Vec<u8> {
    let name_u16: Vec<u8> = name.unwrap_or("").encode_utf16().flat_map(|c| c.to_le_bytes()).collect();
    let voff = (0x18 + name_u16.len() + 7) / 8 * 8;
    let mut a = vec![0u8; voff];
    a[0..4].copy_from_slice(&typ.to_le_bytes());
    a[9] = name_u16.len() as u8 / 2;
    a[0x0A..0x0C].copy_from_slice(&0x18u16.to_le_bytes());
    a[0x0C..0x0E].copy_from_slice(&flags.to_le_bytes());
    a[0x0E..0x10].copy_from_slice(&id.to_le_bytes());
    a[0x10..0x14].copy_from_slice(&(value.len() as u32).to_le_bytes());
    a[0x14..0x16].copy_from_slice(&(voff as u16).to_le_bytes());
    a[0x18..0x18 + name_u16.len()].copy_from_slice(&name_u16);
    a.extend_from_slice(value);
    pad8(&mut a);
    let len = a.len() as u32;
    a[4..8].copy_from_slice(&len.to_le_bytes());
    a
}

fn runlist(runs: &[(u64, Option<u64>)]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut prev = 0i64;
    for &(len, lcn) in runs {
        let lb = bytes_unsigned(len);
        let ob = match lcn {
            Some(l) => {
                let d = l as i64 - prev;
                prev = l as i64;
                bytes_signed(d)
            }
            None => vec![],
        };
        out.push((ob.len() as u8) << 4 | lb.len() as u8);
        out.extend(lb);
        out.extend(ob);
    }
    out.push(0);
    out
}

fn bytes_unsigned(v: u64) -> Vec<u8> {
    let mut b = v.to_le_bytes().to_vec();
    while b.len() > 1 && b[b.len() - 1] == 0 {
        b.pop();
    }
    b
}

fn bytes_signed(v: i64) -> Vec<u8> {
    let mut b = v.to_le_bytes().to_vec();
    while b.len() > 1 {
        let (last, prev) = (b[b.len() - 1], b[b.len() - 2]);
        if (last == 0 && prev & 0x80 == 0) || (last == 0xFF && prev & 0x80 != 0) {
            b.pop();
        } else {
            break;
        }
    }
    b
}

#[allow(clippy::too_many_arguments)]
fn nonresident(typ: u32, id: u16, flags: u16, start_vcn: u64, runs: &[(u64, Option<u64>)], size: u64, init: u64, cu: u8) -> Vec<u8> {
    let rl = runlist(runs);
    let last_vcn = start_vcn + runs.iter().map(|r| r.0).sum::<u64>() - 1;
    let mut a = vec![0u8; 0x40];
    a[0..4].copy_from_slice(&typ.to_le_bytes());
    a[8] = 1;
    a[0x0C..0x0E].copy_from_slice(&flags.to_le_bytes());
    a[0x0E..0x10].copy_from_slice(&id.to_le_bytes());
    a[0x10..0x18].copy_from_slice(&start_vcn.to_le_bytes());
    a[0x18..0x20].copy_from_slice(&last_vcn.to_le_bytes());
    a[0x20..0x22].copy_from_slice(&0x40u16.to_le_bytes());
    a[0x22] = cu;
    a[0x28..0x30].copy_from_slice(&((last_vcn + 1 - start_vcn) * CS as u64).to_le_bytes());
    a[0x30..0x38].copy_from_slice(&size.to_le_bytes());
    a[0x38..0x40].copy_from_slice(&init.to_le_bytes());
    a.extend(rl);
    pad8(&mut a);
    let len = a.len() as u32;
    a[4..8].copy_from_slice(&len.to_le_bytes());
    a
}

fn file_name(parent: u64, ns: u8, name: &str) -> Vec<u8> {
    let units: Vec<u16> = name.encode_utf16().collect();
    let mut v = vec![0u8; 0x42];
    v[0..8].copy_from_slice(&parent.to_le_bytes());
    v[0x40] = units.len() as u8;
    v[0x41] = ns;
    for u in units {
        v.extend_from_slice(&u.to_le_bytes());
    }
    v
}

// 2024-03-11T09:12:00Z
const T: u64 = 1_710_148_320 * 10_000_000 + 116_444_736_000_000_000;

fn std_info() -> Vec<u8> {
    let mut v = vec![0u8; 48];
    v[0..8].copy_from_slice(&T.to_le_bytes());
    v[8..16].copy_from_slice(&(T + 600_000_000).to_le_bytes());
    v
}

fn record(n: u64, seq: u16, in_use: bool, dir: bool, base: u64, attrs: &[Vec<u8>]) -> Vec<u8> {
    let mut r = vec![0u8; RS];
    r[0..4].copy_from_slice(b"FILE");
    r[4..6].copy_from_slice(&0x30u16.to_le_bytes());
    r[6..8].copy_from_slice(&3u16.to_le_bytes());
    r[0x10..0x12].copy_from_slice(&seq.to_le_bytes());
    r[0x14..0x16].copy_from_slice(&0x38u16.to_le_bytes());
    let flags = in_use as u16 | (dir as u16) << 1;
    r[0x16..0x18].copy_from_slice(&flags.to_le_bytes());
    r[0x1C..0x20].copy_from_slice(&(RS as u32).to_le_bytes());
    r[0x20..0x28].copy_from_slice(&base.to_le_bytes());
    r[0x2C..0x30].copy_from_slice(&(n as u32).to_le_bytes());
    let mut off = 0x38;
    for a in attrs {
        r[off..off + a.len()].copy_from_slice(a);
        off += a.len();
    }
    r[off..off + 4].copy_from_slice(&0xFFFF_FFFFu32.to_le_bytes());
    off += 8;
    r[0x18..0x1C].copy_from_slice(&(off as u32).to_le_bytes());
    // Update sequence: USN 0x0007, original sector tails saved in the array.
    r[0x30..0x32].copy_from_slice(&7u16.to_le_bytes());
    for i in 0..RS / 512 {
        let end = (i + 1) * 512;
        let (a, b) = (r[end - 2], r[end - 1]);
        r[0x32 + i * 2] = a;
        r[0x33 + i * 2] = b;
        r[end - 2..end].copy_from_slice(&7u16.to_le_bytes());
    }
    r
}

fn file(n: u64, seq: u16, in_use: bool, parent: u64, name: &str, data: Vec<u8>) -> Vec<u8> {
    record(n, seq, in_use, false, 0, &[resident(0x10, 0, 0, None, &std_info()), resident(0x30, 1, 0, None, &file_name(parent, 1, name)), data])
}

fn dir(n: u64, seq: u16, in_use: bool, parent: u64, name: &str) -> Vec<u8> {
    record(n, seq, in_use, true, 0, &[resident(0x10, 0, 0, None, &std_info()), resident(0x30, 1, 0, None, &file_name(parent, 1, name))])
}

fn pattern(tag: u8, len: usize) -> Vec<u8> {
    (0..len).map(|i| tag ^ (i % 253) as u8).collect()
}

struct Vol {
    disk: Vec<u8>,
    bitmap: Vec<u8>,
}

impl Vol {
    fn put_record(&mut self, n: usize, r: Vec<u8>) {
        let mft_off = n * RS;
        let vcn = (mft_off / CS) as u64;
        let mut acc = 0;
        for (len, lcn) in MFT_RUNS {
            if vcn < acc + len {
                let at = PART + (lcn + vcn - acc) as usize * CS + mft_off % CS;
                self.disk[at..at + RS].copy_from_slice(&r);
                return;
            }
            acc += len;
        }
        panic!("record {n} outside the MFT");
    }
    fn put_cluster(&mut self, lcn: usize, data: &[u8]) {
        let at = PART + lcn * CS;
        self.disk[at..at + data.len()].copy_from_slice(data);
    }
    fn allocate(&mut self, lcn: usize, n: usize) {
        for c in lcn..lcn + n {
            self.bitmap[c / 8] |= 1 << (c % 8);
        }
    }
}

const DOCS: u64 = 16;

fn build() -> Vec<u8> {
    let mut v = Vol { disk: vec![0u8; PART + CLUSTERS * CS], bitmap: vec![0u8; CLUSTERS / 8] };
    let boot = &mut v.disk[PART..PART + 512];
    boot[3..11].copy_from_slice(b"NTFS    ");
    boot[0x0B..0x0D].copy_from_slice(&512u16.to_le_bytes());
    boot[0x0D] = 8;
    boot[0x28..0x30].copy_from_slice(&((CLUSTERS * 8) as u64).to_le_bytes());
    boot[0x30..0x38].copy_from_slice(&4u64.to_le_bytes());
    boot[0x38..0x40].copy_from_slice(&50u64.to_le_bytes());
    boot[0x40] = 0xF6; // -10: 1024-byte records
    boot[510] = 0x55;
    boot[511] = 0xAA;

    let root = fref(5, 5);
    let docs = fref(DOCS, 1);
    v.allocate(0, 16);
    v.allocate(100, 8);

    let mft_runs: Vec<(u64, Option<u64>)> = MFT_RUNS.iter().map(|&(l, c)| (l, Some(c))).collect();
    v.put_record(0, record(0, 1, true, false, 0, &[
        resident(0x30, 1, 0, None, &file_name(root, 3, "$MFT")),
        nonresident(0x80, 2, 0, 0, &mft_runs, 16 * CS as u64, 16 * CS as u64, 0),
    ]));
    v.put_record(5, dir(5, 5, true, root, "."));
    let big = pattern(0xB1, 4 * CS - 100);
    v.put_cluster(200, &big[..2 * CS]);
    v.put_cluster(300, &big[3 * CS..]);
    v.allocate(200, 2);
    v.allocate(300, 1);
    v.put_record(DOCS as usize, dir(DOCS, 1, true, root, "Docs"));
    v.put_record(17, file(17, 1, true, docs, "a.txt", resident(0x80, 2, 0, None, b"hello")));
    v.put_record(18, file(18, 1, true, docs, "big.bin",
        nonresident(0x80, 2, 0, 0, &[(2, Some(200)), (1, None), (1, Some(300))], big.len() as u64, big.len() as u64, 0)));
    // Deleted, clusters still free: fully recoverable.
    v.put_cluster(400, &pattern(0x40, 6000));
    v.put_record(19, file(19, 2, false, docs, "gone.jpg", nonresident(0x80, 2, 0, 0, &[(2, Some(400))], 6000, 6000, 0)));
    // Deleted, clusters since reused by something else.
    v.allocate(500, 1);
    v.put_record(20, file(20, 2, false, docs, "over.bin", nonresident(0x80, 2, 0, 0, &[(1, Some(500))], 3000, 3000, 0)));
    // Deleted folder (sequence bumped on delete) with a deleted file in it.
    v.put_record(21, dir(21, 2, false, root, "Old"));
    v.put_record(22, file(22, 2, false, fref(21, 1), "x.txt", resident(0x80, 2, 0, None, b"old x")));
    // Parent record 24 was reused for a new folder: 23 is orphaned.
    v.put_record(23, file(23, 2, false, fref(24, 1), "orphan.txt", resident(0x80, 2, 0, None, b"lost")));
    v.put_record(24, dir(24, 3, true, root, "NewDir"));
    // DOS alias listed first; the Win32 name must win.
    v.put_record(25, record(25, 1, true, false, 0, &[
        resident(0x30, 1, 0, None, &file_name(docs, 2, "LONGNA~1.TXT")),
        resident(0x30, 3, 0, None, &file_name(docs, 1, "Long Name.txt")),
        resident(0x80, 2, 0, None, b"long"),
    ]));
    // $DATA lives in extension record 27, found via $ATTRIBUTE_LIST.
    let ext_data = pattern(0x27, 5000);
    v.put_cluster(600, &ext_data);
    v.allocate(600, 2);
    let mut list = Vec::new();
    for (typ, rec, id) in [(0x10u32, 26u64, 0u16), (0x30, 26, 1), (0x80, 27, 5)] {
        let mut e = vec![0u8; 0x20];
        e[0..4].copy_from_slice(&typ.to_le_bytes());
        e[4..6].copy_from_slice(&0x20u16.to_le_bytes());
        e[0x10..0x18].copy_from_slice(&fref(rec, 1).to_le_bytes());
        e[0x18..0x1A].copy_from_slice(&id.to_le_bytes());
        list.extend(e);
    }
    v.put_record(26, record(26, 1, true, false, 0, &[
        resident(0x10, 0, 0, None, &std_info()),
        resident(0x20, 4, 0, None, &list),
        resident(0x30, 1, 0, None, &file_name(docs, 1, "listed.dat")),
    ]));
    v.put_record(27, record(27, 1, true, false, fref(26, 1), &[nonresident(0x80, 5, 0, 0, &[(2, Some(600))], 5000, 5000, 0)]));
    // Torn: one sector's tail doesn't match the update sequence number.
    let mut torn = file(28, 1, true, docs, "torn.txt", resident(0x80, 2, 0, None, b"torn"));
    torn[1022] = 0x99;
    v.put_record(28, torn);
    v.put_record(29, file(29, 1, true, docs, "packed.txt", nonresident(0x80, 2, 1, 0, &[(1, Some(700))], 4096, 4096, 4)));
    v.allocate(700, 1);
    v.put_record(30, file(30, 1, true, root, "CON", resident(0x80, 2, 0, None, b"con")));
    v.put_record(31, file(31, 1, true, root, "..", resident(0x80, 2, 0, None, b"dots")));
    // A deleted "a.txt" next to the live one.
    v.put_record(32, file(32, 2, false, docs, "a.txt", resident(0x80, 2, 0, None, b"older hello")));

    let bm = v.bitmap.clone();
    v.put_record(6, record(6, 6, true, false, 0, &[
        resident(0x30, 1, 0, None, &file_name(root, 3, "$Bitmap")),
        resident(0x80, 2, 0, None, &bm),
    ]));
    v.disk
}

fn with_index<R>(disk: Vec<u8>, bad: RangeSet, f: impl FnOnce(&mut Ntfs, &Index) -> R) -> R {
    let mut dev = FakeBlockDevice::new(disk, 512);
    let mut reader = SafeReader::new(&mut dev, bad);
    let mut ntfs = Ntfs::open(&mut reader, PART as u64).unwrap();
    let index = Index::build(&mut ntfs, |_, _| {});
    f(&mut ntfs, &index)
}

#[test]
fn lists_paths_deletion_and_recoverability() {
    with_index(build(), RangeSet::new(), |_, ix| {
        let check = |id: u64, path: Option<&str>, deleted: bool, rec: Recoverable| {
            let e = ix.get(id).unwrap_or_else(|| panic!("record {id} missing"));
            assert_eq!(ix.path(id).as_deref(), path, "path of {id}");
            assert_eq!(!e.in_use, deleted, "deleted flag of {id}");
            assert_eq!(e.recoverable, rec, "recoverable of {id}");
        };
        check(DOCS, Some("/Docs"), false, Recoverable::Full);
        check(17, Some("/Docs/a.txt"), false, Recoverable::Full);
        check(18, Some("/Docs/big.bin"), false, Recoverable::Full);
        check(19, Some("/Docs/gone.jpg"), true, Recoverable::Full);
        check(20, Some("/Docs/over.bin"), true, Recoverable::None);
        check(22, Some("/Old/x.txt"), true, Recoverable::Full);
        check(23, None, true, Recoverable::Full);
        check(25, Some("/Docs/Long Name.txt"), false, Recoverable::Full);
        check(26, Some("/Docs/listed.dat"), false, Recoverable::Full);
        check(29, Some("/Docs/packed.txt"), false, Recoverable::Partial);
        assert!(ix.get(28).is_none(), "torn record must be skipped");
        assert_eq!(ix.get(18).unwrap().size, (4 * CS - 100) as u64);
        assert_eq!(ix.get(26).unwrap().size, 5000);
        assert_eq!(crate::timefmt::filetime_to_rfc3339(ix.get(17).unwrap().created).as_deref(), Some("2024-03-11T09:12:00Z"));
        assert!(!ix.is_listed(0) && !ix.is_listed(5) && !ix.is_listed(6));
        assert!(ix.is_listed(17));
    });
}

#[test]
fn map_holes_make_files_partial() {
    let mut bad = RangeSet::new();
    let at = (PART + 300 * CS) as u64;
    bad.insert(at, at + 512);
    with_index(build(), bad, |_, ix| {
        assert_eq!(ix.get(18).unwrap().recoverable, Recoverable::Partial);
        assert_eq!(ix.get(17).unwrap().recoverable, Recoverable::Full);
    });
}

#[test]
fn torn_mft_record_zero_falls_back_to_the_mirror() {
    let mut disk = build();
    let primary = PART + 4 * CS;
    let rec0 = disk[primary..primary + RS].to_vec();
    disk[PART + 50 * CS..PART + 50 * CS + RS].copy_from_slice(&rec0);
    disk[primary + 510] ^= 0xFF;
    with_index(disk, RangeSet::new(), |_, ix| assert!(ix.get(26).is_some()));
}

fn extract_all(disk: Vec<u8>, bad: RangeSet, ids: &[u64]) -> (Vec<Extracted>, tempfile::TempDir) {
    let out = tempfile::tempdir().unwrap();
    let items = with_index(disk, bad, |ntfs, ix| {
        let mut items = Vec::new();
        extract(ntfs, ix, ids, out.path(), |i| items.push(i.clone()), |_| {}).unwrap();
        items
    });
    (items, out)
}

#[test]
fn extracts_files_folders_orphans_and_hostile_names() {
    let disk = build();
    let big = pattern(0xB1, 4 * CS - 100);
    let (items, out) = extract_all(disk, RangeSet::new(), &[DOCS, 22, 23, 30, 31, 9999]);
    let by_id = |id: u64| items.iter().find(|i| i.id == id).unwrap_or_else(|| panic!("no item {id}"));
    let read = |rel: &str| std::fs::read(out.path().join(rel)).unwrap();

    assert_eq!(by_id(DOCS).path.as_deref(), Some("Docs"));
    assert_eq!(read("Docs/a.txt"), b"hello");
    assert_eq!(by_id(32).path.as_deref(), Some("Docs/a (id 32).txt"));
    assert_eq!(read("Docs/a (id 32).txt"), b"older hello");

    let mut expect_big = big.clone();
    expect_big[2 * CS..3 * CS].fill(0); // sparse run
    assert_eq!(read("Docs/big.bin"), expect_big);
    assert!(by_id(18).complete);
    assert_eq!(read("Docs/gone.jpg"), pattern(0x40, 6000));
    assert_eq!(read("Docs/listed.dat"), pattern(0x27, 5000));

    let over = by_id(20);
    assert!(!over.complete);
    assert!(over.note.as_deref().unwrap().contains("reused"));
    let packed = by_id(29);
    assert!(!packed.complete && packed.size == 0);
    assert!(packed.note.as_deref().unwrap().contains("compressed"));

    assert_eq!(by_id(22).path.as_deref(), Some("Old/x.txt"));
    assert_eq!(by_id(23).path.as_deref(), Some("_orphaned/orphan (id 23).txt"));
    assert_eq!(read("_orphaned/orphan (id 23).txt"), b"lost");
    assert_eq!(by_id(30).path.as_deref(), Some("_CON"));
    assert_eq!(by_id(31).path.as_deref(), Some("__"));
    let missing = by_id(9999);
    assert!(missing.path.is_none() && !missing.complete);
    assert!(items.iter().all(|i| i.id != 28));
}

#[test]
fn unreadable_data_is_zero_filled_and_flagged() {
    let mut bad = RangeSet::new();
    let at = (PART + 200 * CS) as u64;
    bad.insert(at, at + 1024);
    let (items, out) = extract_all(build(), bad, &[18]);
    assert_eq!(items.len(), 1);
    assert!(!items[0].complete);
    assert!(items[0].note.as_deref().unwrap().contains("1024 bytes"));
    let got = std::fs::read(out.path().join("Docs/big.bin")).unwrap();
    assert!(got[..1024].iter().all(|&b| b == 0));
    assert_eq!(got.len(), 4 * CS - 100);
}

#[test]
fn root_extracts_everything_but_metadata() {
    let (items, out) = extract_all(build(), RangeSet::new(), &[5]);
    assert!(items.iter().all(|i| i.id >= 16));
    assert!(out.path().join("Docs/a.txt").exists());
    assert!(!out.path().join("$MFT").exists());
}

#[test]
fn rerunning_extract_never_overwrites_a_different_file() {
    let out = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(out.path().join("Docs")).unwrap();
    std::fs::write(out.path().join("Docs/a.txt"), b"user's own file").unwrap();
    with_index(build(), RangeSet::new(), |ntfs, ix| {
        let mut items = Vec::new();
        extract(ntfs, ix, &[17], out.path(), |i| items.push(i.clone()), |_| {}).unwrap();
        assert_eq!(items[0].path.as_deref(), Some("Docs/a (id 17).txt"));
    });
    assert_eq!(std::fs::read(out.path().join("Docs/a.txt")).unwrap(), b"user's own file");
}
