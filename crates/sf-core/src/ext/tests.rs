//! A hand-built one-group ext volume (1 KiB blocks) mixing ext2 block maps
//! (indirect, double indirect, a hole) with an ext4 extent tree, plus the
//! deleted-name cases: recoverable, wiped, and a rename leftover. A real
//! ext4 image is exercised by hand against Android emulator images.

use super::*;
use crate::block_device::FakeBlockDevice;
use crate::reader::RangeSet;

const BS: usize = 1024;
const PART: usize = 32 * 512;
const BLOCKS: usize = 2048;
const INODE_TABLE: usize = 5;
const MTIME: u32 = 1_710_148_320; // 2024-03-11T09:12:00Z

fn text(tag: &str, n: usize) -> Vec<u8> {
    (0..).flat_map(|i| format!("{tag} line {i}\n").into_bytes()).take(n).collect()
}

struct Img {
    d: Vec<u8>,
}

impl Img {
    fn new() -> Self {
        let mut d = vec![0u8; PART + BLOCKS * BS];
        let sb = &mut d[PART + 1024..PART + 2048];
        sb[0..4].copy_from_slice(&64u32.to_le_bytes()); // inodes
        sb[4..8].copy_from_slice(&(BLOCKS as u32).to_le_bytes());
        sb[20..24].copy_from_slice(&1u32.to_le_bytes()); // first data block
        sb[32..36].copy_from_slice(&8192u32.to_le_bytes()); // blocks per group
        sb[40..44].copy_from_slice(&64u32.to_le_bytes()); // inodes per group
        sb[56..58].copy_from_slice(&0xEF53u16.to_le_bytes());
        sb[76..80].copy_from_slice(&1u32.to_le_bytes()); // dynamic rev
        sb[88..90].copy_from_slice(&128u16.to_le_bytes());
        sb[96..100].copy_from_slice(&(INCOMPAT_FILETYPE | 0x40).to_le_bytes());
        let gd = PART + 2 * BS;
        d[gd..gd + 4].copy_from_slice(&3u32.to_le_bytes()); // block bitmap
        d[gd + 8..gd + 12].copy_from_slice(&(INODE_TABLE as u32).to_le_bytes());
        let mut img = Img { d };
        img.used(0..20);
        img
    }

    fn block(&mut self, b: usize) -> &mut [u8] {
        &mut self.d[PART + b * BS..PART + (b + 1) * BS]
    }

    fn used(&mut self, blocks: impl IntoIterator<Item = usize>) {
        for b in blocks {
            // Bit 0 is block 1 (the first data block).
            let rel = b.saturating_sub(1);
            self.d[PART + 3 * BS + rel / 8] |= 1 << (rel % 8);
        }
    }

    fn put(&mut self, blocks: &[usize], data: &[u8]) {
        for (i, &b) in blocks.iter().enumerate() {
            let part = &data[(i * BS).min(data.len())..((i + 1) * BS).min(data.len())];
            self.block(b)[..part.len()].copy_from_slice(part);
        }
    }

    fn inode(&mut self, n: usize, mode: u16, size: u64, links: u16, flags: u32, block: &[u8]) {
        let o = PART + INODE_TABLE * BS + (n - 1) * 128;
        let i = &mut self.d[o..o + 128];
        i[0..2].copy_from_slice(&mode.to_le_bytes());
        i[4..8].copy_from_slice(&(size as u32).to_le_bytes());
        i[16..20].copy_from_slice(&MTIME.to_le_bytes());
        if links == 0 {
            i[20..24].copy_from_slice(&MTIME.to_le_bytes()); // dtime
        }
        i[26..28].copy_from_slice(&links.to_le_bytes());
        i[32..36].copy_from_slice(&flags.to_le_bytes());
        i[40..40 + block.len()].copy_from_slice(block);
        i[108..112].copy_from_slice(&((size >> 32) as u32).to_le_bytes());
    }
}

fn ptrs(p: &[u32]) -> Vec<u8> {
    p.iter().flat_map(|x| x.to_le_bytes()).collect()
}

/// Directory entry; `rec_len` 0 = just big enough.
fn dirent(ino: u32, name: &str, ftype: u8, rec_len: usize) -> Vec<u8> {
    let need = (8 + name.len() + 3) & !3;
    let mut e = vec![0u8; rec_len.max(need)];
    e[0..4].copy_from_slice(&ino.to_le_bytes());
    let len = e.len() as u16;
    e[4..6].copy_from_slice(&len.to_le_bytes());
    e[6] = name.len() as u8;
    e[7] = ftype;
    e[8..8 + name.len()].copy_from_slice(name.as_bytes());
    e
}

fn extent_header(entries: u16, depth: u16) -> Vec<u8> {
    let mut h = vec![0u8; 12];
    h[0..2].copy_from_slice(&0xF30Au16.to_le_bytes());
    h[2..4].copy_from_slice(&entries.to_le_bytes());
    h[4..6].copy_from_slice(&4u16.to_le_bytes());
    h[6..8].copy_from_slice(&depth.to_le_bytes());
    h
}

#[test]
fn ext_lists_and_recovers_block_maps_extents_and_deleted_names() {
    let mut img = Img::new();
    const DIR: u16 = 0x41ED;
    const REG: u16 = 0x81A4;

    let small = text("small", 1500);
    img.put(&[30, 31], &small);
    img.used([30, 31]);
    img.inode(12, REG, 1500, 1, 0, &ptrs(&[30, 31]));

    // 280 blocks: 12 direct, 256 through the indirect block, 12 through
    // the double indirect one. Logical block 20 is a hole.
    let mut big = text("big", 280 * BS - 100);
    big[20 * BS..21 * BS].fill(0);
    let data: Vec<usize> = (100..380).collect();
    let mut direct: Vec<u32> = data[..12].iter().map(|&b| b as u32).collect();
    direct.extend([98, 97, 0]);
    let mut ind: Vec<u32> = data[12..268].iter().map(|&b| b as u32).collect();
    ind[20 - 12] = 0;
    img.block(98).copy_from_slice(&ptrs(&ind));
    img.block(97)[..4].copy_from_slice(&96u32.to_le_bytes());
    img.block(96)[..48].copy_from_slice(&ptrs(&data[268..].iter().map(|&b| b as u32).collect::<Vec<_>>()));
    for (i, &b) in data.iter().enumerate() {
        if i != 20 {
            img.block(b).copy_from_slice(&big[i * BS..((i + 1) * BS).min(big.len())].iter().copied().chain(std::iter::repeat(0)).take(BS).collect::<Vec<_>>());
        }
    }
    img.used(96..380);
    img.inode(13, REG, big.len() as u64, 1, 0, &ptrs(&direct));

    // ext4 extent tree, depth 1: index → leaf block 50 → two extents.
    let ext = text("extents", 3000);
    img.put(&[60, 61], &ext[..2 * BS]);
    img.put(&[70], &ext[2 * BS..]);
    let mut leaf = extent_header(2, 0);
    for (lblk, len, phys) in [(0u32, 2u16, 60u32), (2, 1, 70)] {
        leaf.extend(lblk.to_le_bytes());
        leaf.extend(len.to_le_bytes());
        leaf.extend(0u16.to_le_bytes());
        leaf.extend(phys.to_le_bytes());
    }
    img.block(50)[..leaf.len()].copy_from_slice(&leaf);
    let mut root_node = extent_header(1, 1);
    root_node.extend(0u32.to_le_bytes());
    root_node.extend(50u32.to_le_bytes());
    root_node.extend([0u8; 4]);
    img.used([50, 60, 61, 70]);
    img.inode(15, REG, 3000, 1, FL_EXTENTS, &root_node);

    // Deleted, data intact and free.
    let gone = text("gone", 500);
    img.put(&[80], &gone);
    img.inode(16, REG, 500, 0, 0, &ptrs(&[80]));
    // Deleted with its pointers wiped (what ext3/ext4 normally do).
    img.inode(17, REG, 900, 0, 0, &[]);

    let mut sub = dirent(14, ".", 2, 0);
    sub.extend(dirent(2, "..", 2, 0));
    sub.extend(dirent(15, "ext.txt", 1, BS - sub.len()));
    img.block(40).copy_from_slice(&sub);
    img.used([40]);
    img.inode(14, DIR, BS as u64, 2, 0, &ptrs(&[40]));

    // "small.txt" owns the slack where three deleted names still sit.
    let mut root = dirent(2, ".", 2, 0);
    root.extend(dirent(2, "..", 2, 0));
    let mut slack = dirent(12, "small.txt", 1, 0);
    slack.extend(dirent(16, "gone.txt", 1, 0));
    slack.extend(dirent(12, "oldname.txt", 1, 0)); // a rename leftover: inode still live
    slack.extend(dirent(17, "wiped.txt", 1, 0));
    let len = slack.len() as u16;
    slack[4..6].copy_from_slice(&len.to_le_bytes());
    root.extend(slack);
    root.extend(dirent(13, "big.bin", 1, 0));
    let rest = BS - root.len();
    root.extend(dirent(14, "sub", 2, rest));
    img.block(20).copy_from_slice(&root);
    img.used([20]);
    img.inode(2, DIR, BS as u64, 3, 0, &ptrs(&[20]));

    let mut dev = FakeBlockDevice::new(img.d, 512);
    let mut reader = SafeReader::new(&mut dev, RangeSet::new());
    let mut vol = Ext::open(&mut reader, PART as u64).unwrap();
    let index = vol.index(&mut |_, _| {});
    let by_path: HashMap<String, u64> =
        index.ids().into_iter().filter(|&i| index.is_listed(i)).filter_map(|i| Some((index.path(i)?, i))).collect();
    let mut paths: Vec<&str> = by_path.keys().map(|s| s.as_str()).collect();
    paths.sort();
    assert_eq!(paths, ["/big.bin", "/gone.txt", "/small.txt", "/sub", "/sub/ext.txt", "/wiped.txt"]);

    for (path, want, live) in [("/small.txt", &small, true), ("/big.bin", &big, true), ("/sub/ext.txt", &ext, true), ("/gone.txt", &gone, false)] {
        let id = by_path[path];
        let e = index.get(id).unwrap();
        assert_eq!(e.recoverable, Recoverable::Full, "{path}");
        assert_eq!(e.in_use, live, "{path}");
        let mut out = Vec::new();
        let (n, notes) = vol.copy_file(&index, id, &mut out).unwrap();
        assert_eq!(n as usize, want.len(), "{path}");
        assert!(&out == want, "{path} content differs");
        assert!(notes.is_empty(), "{path}: {notes:?}");
    }
    let wiped = by_path["/wiped.txt"];
    assert_eq!(index.get(wiped).unwrap().recoverable, Recoverable::None);
    let (_, notes) = vol.copy_file(&index, wiped, &mut Vec::new()).unwrap();
    assert!(notes[0].contains("wiped"), "{notes:?}");
    assert_eq!(
        crate::timefmt::filetime_to_rfc3339(index.get(by_path["/small.txt"]).unwrap().modified).as_deref(),
        Some("2024-03-11T09:12:00Z")
    );
}
