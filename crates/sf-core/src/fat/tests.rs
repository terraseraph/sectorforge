//! Hand-built FAT16, FAT32 and exFAT volumes covering what's hard to get
//! from a formatter on demand: fragmented chains, VFAT long names, deleted
//! files and folders, reused clusters, a bad sector in the first FAT copy.

use super::dir::{lfn_checksum, set_checksum};
use super::*;
use crate::block_device::FakeBlockDevice;
use crate::reader::RangeSet;

const PART: usize = 64 * 512;
const BPS: usize = 512;

fn text(tag: &str, n: usize) -> Vec<u8> {
    (0..).flat_map(|i| format!("{tag} line {i}\n").into_bytes()).take(n).collect()
}

/// A FAT16/32 volume with 512-byte clusters, two FAT copies.
struct Img {
    d: Vec<u8>,
    fat32: bool,
    fat_off: usize,
    fat_len: usize,
    heap: usize,
    root_off: usize,
}

impl Img {
    fn new(fat32: bool) -> Self {
        let clusters: usize = if fat32 { 66_000 } else { 4200 };
        let (reserved, root_ents) = if fat32 { (32, 0) } else { (1, 64) };
        let fat_len = (clusters + 2) * if fat32 { 4 } else { 2 };
        let fat_sectors = fat_len.div_ceil(BPS);
        let root_sectors = root_ents * 32 / BPS;
        let first_data = reserved + 2 * fat_sectors + root_sectors;
        let total = first_data + clusters;
        let mut d = vec![0u8; PART + total * BPS];
        let b = &mut d[PART..PART + 512];
        b[0..3].copy_from_slice(&[0xEB, 0x3C, 0x90]);
        b[3..11].copy_from_slice(b"MSDOS5.0");
        b[11..13].copy_from_slice(&(BPS as u16).to_le_bytes());
        b[13] = 1;
        b[14..16].copy_from_slice(&(reserved as u16).to_le_bytes());
        b[16] = 2;
        b[17..19].copy_from_slice(&(root_ents as u16).to_le_bytes());
        b[21] = 0xF8;
        b[32..36].copy_from_slice(&(total as u32).to_le_bytes());
        if fat32 {
            b[36..40].copy_from_slice(&(fat_sectors as u32).to_le_bytes());
            b[44..48].copy_from_slice(&2u32.to_le_bytes());
            b[82..90].copy_from_slice(b"FAT32   ");
        } else {
            b[22..24].copy_from_slice(&(fat_sectors as u16).to_le_bytes());
            b[54..62].copy_from_slice(b"FAT16   ");
        }
        b[510] = 0x55;
        b[511] = 0xAA;
        let mut img = Img {
            d,
            fat32,
            fat_off: PART + reserved * BPS,
            fat_len: fat_sectors * BPS,
            heap: PART + first_data * BPS,
            root_off: PART + (reserved + 2 * fat_sectors) * BPS,
        };
        img.set_fat(0, 0xFFF8);
        img.set_fat(1, 0xFFFF);
        if fat32 {
            img.set_fat(2, 0x0FFF_FFFF); // root directory
        }
        img
    }

    fn set_fat(&mut self, c: u32, v: u32) {
        for copy in 0..2 {
            let o = self.fat_off + copy * self.fat_len;
            if self.fat32 {
                self.d[o + c as usize * 4..][..4].copy_from_slice(&v.to_le_bytes());
            } else {
                self.d[o + c as usize * 2..][..2].copy_from_slice(&(v as u16).to_le_bytes());
            }
        }
    }

    fn eoc(&self) -> u32 {
        if self.fat32 { 0x0FFF_FFFF } else { 0xFFFF }
    }

    fn cluster(&mut self, c: u32) -> &mut [u8] {
        let o = self.heap + (c as usize - 2) * BPS;
        &mut self.d[o..o + BPS]
    }

    /// Writes `data` across `clusters`; chains them unless `deleted`.
    fn put(&mut self, clusters: &[u32], data: &[u8], chain: bool) {
        for (i, &c) in clusters.iter().enumerate() {
            let part = &data[(i * BPS).min(data.len())..((i + 1) * BPS).min(data.len())];
            self.cluster(c)[..part.len()].copy_from_slice(part);
            if chain {
                let next = clusters.get(i + 1).copied().unwrap_or(self.eoc());
                self.set_fat(c, next);
            }
        }
    }

    /// Writes 32-byte entries into the root (FixedRoot or cluster 2).
    fn root(&mut self, at: usize, entries: &[[u8; 32]]) {
        let base = if self.fat32 { self.heap } else { self.root_off };
        for (i, e) in entries.iter().enumerate() {
            self.d[base + (at + i) * 32..][..32].copy_from_slice(e);
        }
    }

    fn dir(&mut self, c: u32, entries: &[[u8; 32]]) {
        for (i, e) in entries.iter().enumerate() {
            self.cluster(c)[i * 32..(i + 1) * 32].copy_from_slice(e);
        }
    }
}

fn short(name: &[u8; 11], attr: u8, first: u32, size: u32) -> [u8; 32] {
    let mut e = [0u8; 32];
    e[0..11].copy_from_slice(name);
    e[11] = attr;
    e[20..22].copy_from_slice(&((first >> 16) as u16).to_le_bytes());
    e[26..28].copy_from_slice(&(first as u16).to_le_bytes());
    e[28..32].copy_from_slice(&size.to_le_bytes());
    // 2024-03-11 09:12:00
    let date: u16 = (2024 - 1980) << 9 | 3 << 5 | 11;
    e[22..24].copy_from_slice(&(9u16 << 11 | 12 << 5).to_le_bytes());
    e[24..26].copy_from_slice(&date.to_le_bytes());
    e
}

/// LFN entries in disk order (last part first) for 8.3 entry `sfn`.
fn lfn(name: &str, sfn: &[u8; 11]) -> Vec<[u8; 32]> {
    let mut units: Vec<u16> = name.encode_utf16().collect();
    units.push(0);
    while !units.len().is_multiple_of(13) {
        units.push(0xFFFF);
    }
    let n = units.len() / 13;
    let sum = lfn_checksum(sfn);
    (0..n)
        .rev()
        .map(|i| {
            let mut e = [0u8; 32];
            e[0] = (i + 1) as u8 | if i == n - 1 { 0x40 } else { 0 };
            e[11] = 0x0F;
            e[13] = sum;
            let part = &units[i * 13..i * 13 + 13];
            let bytes: Vec<u8> = part.iter().flat_map(|u| u.to_le_bytes()).collect();
            e[1..11].copy_from_slice(&bytes[0..10]);
            e[14..26].copy_from_slice(&bytes[10..22]);
            e[28..32].copy_from_slice(&bytes[22..26]);
            e
        })
        .collect()
}

fn deleted(mut entries: Vec<[u8; 32]>) -> Vec<[u8; 32]> {
    for e in &mut entries {
        e[0] = 0xE5;
    }
    entries
}

fn dots(me: u32, parent: u32) -> Vec<[u8; 32]> {
    vec![short(b".          ", 0x10, me, 0), short(b"..         ", 0x10, parent, 0)]
}

struct Found {
    index: Index,
    by_path: HashMap<String, u64>,
}

impl Found {
    fn id(&self, path: &str) -> u64 {
        *self.by_path.get(path).unwrap_or_else(|| panic!("{path} not listed; have {:?}", self.by_path.keys()))
    }
    fn entry(&self, path: &str) -> &IndexEntry {
        self.index.get(self.id(path)).unwrap()
    }
}

fn open(dev: &mut FakeBlockDevice, f: impl FnOnce(&mut Fat, Found)) {
    open_with(dev, RangeSet::new(), f)
}

fn open_with(dev: &mut FakeBlockDevice, bad: RangeSet, f: impl FnOnce(&mut Fat, Found)) {
    let mut reader = SafeReader::new(dev, bad);
    let mut vol = Fat::open(&mut reader, PART as u64).unwrap();
    let index = vol.index(&mut |_, _| {});
    let by_path = index.ids().into_iter().filter(|&i| index.is_listed(i)).filter_map(|i| Some((index.path(i)?, i))).collect();
    f(&mut vol, Found { index, by_path });
}

fn read(vol: &mut Fat, found: &Found, path: &str) -> (Vec<u8>, Vec<String>) {
    let mut out = Vec::new();
    let (n, notes) = vol.copy_file(&found.index, found.id(path), &mut out).unwrap();
    assert_eq!(n as usize, out.len());
    (out, notes)
}

/// The scenario shared by FAT16 and FAT32.
fn build(fat32: bool) -> (Img, HashMap<&'static str, Vec<u8>>) {
    let mut img = Img::new(fat32);
    let mut want = HashMap::new();
    let report = text("report", 1300);
    img.put(&[10, 11, 20], &report, true); // fragmented
    let a = text("a", 100);
    img.put(&[31], &a, true);
    img.put(&[30], &[], true);
    let mut docs = dots(30, 0);
    docs.push(short(b"A       TXT", 0x20, 31, 100));
    img.dir(30, &docs);
    let gone = text("gone", 1500);
    img.put(&[40, 41, 42], &gone, false);
    // A deleted file whose second cluster now belongs to someone else.
    img.put(&[50, 51], &text("overwritten", 1024), false);
    img.put(&[51], &text("newer", 300), true);
    // A deleted folder: its file's entry still looks live but its chain
    // was wiped with the folder's.
    let memo = text("memo", 200);
    img.put(&[61], &memo, false);
    let mut old = dots(60, 0);
    old.push(short(b"MEMO    TXT", 0x20, 61, 200));
    img.dir(60, &old);
    let unicode = text("unicode", 50);
    img.put(&[70], &unicode, true);

    let rep_sfn = b"QUARTE~1TXT";
    let mut root = vec![short(b"SFTEST     ", 0x08, 0, 0)];
    root.extend(lfn("Quarterly Report.txt", rep_sfn));
    root.push(short(rep_sfn, 0x20, 10, 1300));
    root.push(short(b"DOCS       ", 0x10, 30, 0));
    let gone_sfn = b"DELETE~1TXT";
    let mut g = lfn("deleted file.txt", gone_sfn);
    g.push(short(gone_sfn, 0x20, 40, 1500));
    root.extend(deleted(g));
    root.extend(deleted(vec![short(b"REUSED  BIN", 0x20, 50, 1024)]));
    root.push(short(b"NEWER   TXT", 0x20, 51, 300));
    root.extend(deleted(vec![short(b"OLD        ", 0x10, 60, 0)]));
    let u_sfn = b"CAFUNI~1TXT";
    root.extend(lfn("caf\u{e9} \u{fc}n\u{ef}code.txt", u_sfn));
    root.push(short(u_sfn, 0x20, 70, 50));
    // Junk that isn't an entry must be skipped, not listed.
    let mut junk = short(b"BAD\x01NAME TX", 0x20, 99, 10);
    junk[11] = 0x20;
    root.push(junk);
    img.root(0, &root);

    want.insert("/Quarterly Report.txt", report);
    want.insert("/DOCS/A.TXT", a);
    want.insert("/deleted file.txt", gone);
    want.insert("/_LD/MEMO.TXT", memo);
    want.insert("/caf\u{e9} \u{fc}n\u{ef}code.txt", unicode);
    (img, want)
}

fn check_scenario(fat32: bool) {
    let (img, want) = build(fat32);
    let mut dev = FakeBlockDevice::new(img.d, 512);
    open(&mut dev, |vol, found| {
        assert_eq!(vol.kind, if fat32 { FatKind::Fat32 } else { FatKind::Fat16 });
        for (path, data) in &want {
            let (got, notes) = read(vol, &found, path);
            assert_eq!(&got, data, "{path}");
            assert!(notes.is_empty(), "{path}: {notes:?}");
            assert_eq!(found.entry(path).recoverable, Recoverable::Full, "{path}");
        }
        assert!(found.entry("/Quarterly Report.txt").in_use);
        assert!(!found.entry("/deleted file.txt").in_use);
        assert!(!found.entry("/_LD").in_use && found.entry("/_LD").is_dir);
        assert!(!found.entry("/_LD/MEMO.TXT").in_use);
        assert_eq!(crate::timefmt::filetime_to_rfc3339(found.entry("/DOCS/A.TXT").modified).as_deref(), Some("2024-03-11T09:12:00Z"));

        let reused = found.entry("/_EUSED.BIN");
        assert_eq!(reused.recoverable, Recoverable::Partial);
        let (_, notes) = read(vol, &found, "/_EUSED.BIN");
        assert!(notes.iter().any(|n| n.contains("reused")), "{notes:?}");

        // Volume label and junk aren't files.
        assert_eq!(found.by_path.len(), 9, "{:?}", found.by_path.keys());
    });
}

#[test]
fn fat16_lists_and_recovers_live_and_deleted_files() {
    check_scenario(false);
}

#[test]
fn fat32_lists_and_recovers_live_and_deleted_files() {
    check_scenario(true);
}

#[test]
fn bad_first_fat_copy_falls_back_to_second() {
    let (img, want) = build(false);
    let fat0 = img.fat_off as u64;
    // Corrupt nothing, just make copy 1's first sector unreadable: the
    // fragmented chain (clusters 10, 11, 20) lives there.
    let mut dev = FakeBlockDevice::new(img.d, 512);
    dev.mark_bad(fat0, fat0 + 512);
    open(&mut dev, |vol, found| {
        let (got, notes) = read(vol, &found, "/Quarterly Report.txt");
        assert_eq!(got, want["/Quarterly Report.txt"]);
        assert!(notes.is_empty(), "{notes:?}");
    });
}

#[test]
fn unreadable_data_is_reported() {
    let (img, want) = build(false);
    let c11 = (img.heap + 9 * BPS) as u64;
    let mut dev = FakeBlockDevice::new(img.d, 512);
    dev.mark_bad(c11, c11 + 512);
    // Known from the imaging map, so the listing can say so up front.
    let mut bad = RangeSet::new();
    bad.insert(c11, c11 + 512);
    open_with(&mut dev, bad, |vol, found| {
        assert_eq!(found.entry("/Quarterly Report.txt").recoverable, Recoverable::Partial);
        let (got, notes) = read(vol, &found, "/Quarterly Report.txt");
        assert_eq!(got.len(), want["/Quarterly Report.txt"].len());
        assert!(got[512..1024].iter().all(|&b| b == 0));
        assert!(notes[0].contains("512 bytes could not be read"), "{notes:?}");
    });
}

// ---- exFAT ----

const EX_FAT_OFF: usize = 24;
const EX_HEAP: usize = 32;
const EX_CLUSTERS: u32 = 200;

struct Ex {
    d: Vec<u8>,
    bitmap: Vec<u8>,
}

impl Ex {
    fn new() -> Self {
        let total = EX_HEAP + EX_CLUSTERS as usize;
        let mut d = vec![0u8; PART + total * BPS];
        let b = &mut d[PART..PART + 512];
        b[3..11].copy_from_slice(b"EXFAT   ");
        b[0x48..0x50].copy_from_slice(&(total as u64).to_le_bytes());
        b[0x50..0x54].copy_from_slice(&(EX_FAT_OFF as u32).to_le_bytes());
        b[0x54..0x58].copy_from_slice(&8u32.to_le_bytes());
        b[0x58..0x5C].copy_from_slice(&(EX_HEAP as u32).to_le_bytes());
        b[0x5C..0x60].copy_from_slice(&EX_CLUSTERS.to_le_bytes());
        b[0x60..0x64].copy_from_slice(&4u32.to_le_bytes());
        b[0x6C] = 9;
        b[0x6D] = 0;
        b[0x6E] = 1;
        b[510] = 0x55;
        b[511] = 0xAA;
        let mut ex = Ex { d, bitmap: vec![0u8; (EX_CLUSTERS as usize).div_ceil(8)] };
        // The root takes two clusters.
        ex.set_fat(4, 5);
        ex.set_fat(5, u32::MAX);
        ex.alloc(&[2, 3, 4, 5]);
        ex
    }

    fn set_fat(&mut self, c: u32, v: u32) {
        let o = PART + EX_FAT_OFF * BPS + c as usize * 4;
        self.d[o..o + 4].copy_from_slice(&v.to_le_bytes());
    }

    fn alloc(&mut self, cs: &[u32]) {
        for &c in cs {
            self.bitmap[(c - 2) as usize / 8] |= 1 << ((c - 2) % 8);
        }
    }

    fn cluster(&mut self, c: u32) -> &mut [u8] {
        let o = PART + EX_HEAP * BPS + (c as usize - 2) * BPS;
        &mut self.d[o..o + BPS]
    }

    fn put(&mut self, clusters: &[u32], data: &[u8]) {
        for (i, &c) in clusters.iter().enumerate() {
            let part = &data[(i * BPS).min(data.len())..((i + 1) * BPS).min(data.len())];
            self.cluster(c)[..part.len()].copy_from_slice(part);
        }
    }

    /// Writes entries from cluster `c` on, spilling into the next clusters.
    fn write_dir(&mut self, c: u32, entries: &[[u8; 32]]) {
        let o = PART + EX_HEAP * BPS + (c as usize - 2) * BPS;
        self.d[o..o + entries.len() * 32].copy_from_slice(&entries.concat());
    }

    fn finish(mut self) -> Vec<u8> {
        let bm = self.bitmap.clone();
        self.cluster(2)[..bm.len()].copy_from_slice(&bm);
        self.d
    }
}

/// A file entry set. `in_use` false = deleted (bit 7 cleared everywhere).
fn ex_file(name: &str, dir: bool, first: u32, size: u64, no_chain: bool, in_use: bool) -> Vec<[u8; 32]> {
    let units: Vec<u16> = name.encode_utf16().collect();
    let names = units.len().div_ceil(15);
    let mut set = vec![[0u8; 32]; 2 + names];
    set[0][0] = 0x85;
    set[0][1] = (1 + names) as u8;
    set[0][4] = if dir { 0x10 } else { 0x20 };
    // 2024-03-11 09:12:00 local at UTC+10, so 2024-03-10 23:12:00Z.
    let ts: u32 = ((2024 - 1980) << 25) | 3 << 21 | 11 << 16 | 9 << 11 | 12 << 5;
    set[0][12..16].copy_from_slice(&ts.to_le_bytes());
    set[0][23] = 0x80 | 40;
    set[1][0] = 0xC0;
    set[1][1] = 1 | if no_chain { 2 } else { 0 };
    set[1][3] = units.len() as u8;
    set[1][8..16].copy_from_slice(&size.to_le_bytes());
    set[1][20..24].copy_from_slice(&first.to_le_bytes());
    set[1][24..32].copy_from_slice(&size.to_le_bytes());
    for (i, chunk) in units.chunks(15).enumerate() {
        set[2 + i][0] = 0xC1;
        for (j, u) in chunk.iter().enumerate() {
            set[2 + i][2 + j * 2..4 + j * 2].copy_from_slice(&u.to_le_bytes());
        }
    }
    let flat: Vec<u8> = set.concat();
    let sum = set_checksum(&flat);
    set[0][2..4].copy_from_slice(&sum.to_le_bytes());
    if !in_use {
        for e in &mut set {
            e[0] &= 0x7F;
        }
    }
    set
}

#[test]
fn exfat_lists_and_recovers_live_and_deleted_files() {
    let mut ex = Ex::new();
    let mut root = Vec::new();
    let mut bm = [0u8; 32];
    bm[0] = 0x81;
    bm[20..24].copy_from_slice(&2u32.to_le_bytes());
    bm[24..32].copy_from_slice(&((EX_CLUSTERS as u64).div_ceil(8)).to_le_bytes());
    root.push(bm);

    let hello = text("hello", 1200);
    ex.put(&[10, 11, 12], &hello);
    ex.alloc(&[10, 11, 12]);
    root.extend(ex_file("hello.txt", false, 10, 1200, true, true));

    let frag = text("frag", 1000);
    ex.put(&[20, 25], &frag);
    ex.set_fat(20, 25);
    ex.set_fat(25, u32::MAX);
    ex.alloc(&[20, 25]);
    root.extend(ex_file("a file with a rather long name.bin", false, 20, 1000, false, true));

    let gone = text("gone", 700);
    ex.put(&[40, 41], &gone);
    root.extend(ex_file("gone.txt", false, 40, 700, true, false));

    // Deleted, and its space taken by "newer.txt".
    ex.put(&[45], &text("newer", 100));
    ex.alloc(&[45]);
    root.extend(ex_file("old.txt", false, 45, 300, true, false));
    root.extend(ex_file("newer.txt", false, 45, 100, true, true));

    let inner = text("inner", 10);
    ex.put(&[51], &inner);
    ex.alloc(&[50, 51]);
    ex.write_dir(50, &ex_file("inner.txt", false, 51, 10, true, true));
    root.extend(ex_file("Sub", true, 50, 512, true, true));

    // A set with a broken checksum is skipped.
    let mut bad = ex_file("corrupt.txt", false, 60, 10, true, true);
    bad[0][2] ^= 1;
    root.extend(bad);

    ex.write_dir(4, &root);
    let mut dev = FakeBlockDevice::new(ex.finish(), 512);
    open(&mut dev, |vol, found| {
        assert_eq!(vol.kind, FatKind::ExFat);
        for (path, data) in [
            ("/hello.txt", &hello),
            ("/a file with a rather long name.bin", &frag),
            ("/gone.txt", &gone),
            ("/Sub/inner.txt", &inner),
        ] {
            let (got, notes) = read(vol, &found, path);
            assert_eq!(&got, data, "{path}");
            assert!(notes.is_empty(), "{path}: {notes:?}");
            assert_eq!(found.entry(path).recoverable, Recoverable::Full, "{path}");
        }
        assert!(!found.entry("/gone.txt").in_use);
        assert_eq!(found.entry("/old.txt").recoverable, Recoverable::None);
        assert_eq!(
            crate::timefmt::filetime_to_rfc3339(found.entry("/hello.txt").modified).as_deref(),
            Some("2024-03-10T23:12:00Z")
        );
        assert!(!found.by_path.contains_key("/corrupt.txt"));
        assert_eq!(found.by_path.len(), 7, "{:?}", found.by_path.keys());
    });
}
