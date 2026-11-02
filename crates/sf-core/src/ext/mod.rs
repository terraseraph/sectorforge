//! ext2/3/4 file listing and recovery through the same [`Index`] and
//! [`Volume`] as NTFS.
//!
//! Live files are read through their extent tree or block map. Deleting a
//! file on ext3/ext4 normally wipes its block pointers, so deleted names
//! (found in the slack space of directory entries) are only recoverable
//! when their freed inode still maps its data; otherwise they're listed as
//! not recoverable rather than hidden. Names whose inode is in use again
//! are leftovers from renames or reuse and are skipped.

#[cfg(test)]
mod tests;

use std::collections::{HashMap, HashSet, VecDeque};
use std::io::Write;

use crate::ntfs::{unreadable_note, Index, IndexEntry, Recoverable, Volume, FIRST_USER_RECORD, REUSED_NOTE, ROOT};
use crate::reader::SafeReader;
use crate::timefmt::unix_to_filetime;

const ROOT_INODE: u32 = 2;
const COPY_CHUNK: u64 = 4 * 1024 * 1024;
const MAX_DIR_BYTES: u64 = 64 * 1024 * 1024;
const MAX_EXTENT_NODES: usize = 1 << 16;

const INCOMPAT_FILETYPE: u32 = 0x2;
const INCOMPAT_JOURNAL_DEV: u32 = 0x8;
const INCOMPAT_META_BG: u32 = 0x10;
const INCOMPAT_64BIT: u32 = 0x80;
const RO_COMPAT_SPARSE_SUPER: u32 = 0x1;

const FL_ENCRYPT: u32 = 0x800;
const FL_INDEX: u32 = 0x1000;
const FL_EXTENTS: u32 = 0x8_0000;
const FL_INLINE_DATA: u32 = 0x1000_0000;

const S_IFMT: u16 = 0xF000;
const S_IFDIR: u16 = 0x4000;
const S_IFREG: u16 = 0x8000;

fn u16_at(b: &[u8], o: usize) -> u16 {
    u16::from_le_bytes([b[o], b[o + 1]])
}
fn u32_at(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes(b[o..o + 4].try_into().unwrap())
}

#[derive(Debug, Clone)]
struct Inode {
    mode: u16,
    size: u64,
    links: u16,
    flags: u32,
    mtime: u32,
    crtime: Option<u32>,
    block: [u8; 60],
}

impl Inode {
    fn is_dir(&self) -> bool {
        self.mode & S_IFMT == S_IFDIR
    }
}

/// Where a file's blocks are: (first logical block, first physical block,
/// count). Holes are simply absent.
#[derive(Debug, Clone, Default)]
struct Map {
    extents: Vec<(u64, u64, u64)>,
    /// Data stored in the inode itself (inline data).
    inline: Option<Vec<u8>>,
    /// Part of the map couldn't be read or made no sense.
    incomplete: bool,
}

pub struct Ext<'r, 'a> {
    reader: &'r mut SafeReader<'a>,
    part_off: u64,
    block_size: u64,
    blocks: u64,
    first_data_block: u64,
    blocks_per_group: u64,
    inodes_per_group: u64,
    inode_size: u64,
    inodes: u64,
    filetype: bool,
    /// (block bitmap block, inode table block) per group.
    groups: Vec<(u64, u64)>,
    bitmap: Option<Vec<u8>>,
    /// Index id → (inode number, deleted).
    files: HashMap<u64, (u32, bool)>,
}

impl<'r, 'a> Ext<'r, 'a> {
    pub fn open(reader: &'r mut SafeReader<'a>, part_off: u64) -> Result<Self, String> {
        let mut sb = vec![0u8; 1024];
        if reader.read(part_off + 1024, &mut sb) > 0 {
            return Err("the ext superblock can't be read".into());
        }
        if u16_at(&sb, 56) != 0xEF53 || u32_at(&sb, 24) > 6 {
            return Err("no valid ext superblock at this partition offset".into());
        }
        let incompat = u32_at(&sb, 96);
        if incompat & INCOMPAT_JOURNAL_DEV != 0 {
            return Err("this is an external ext journal device, which holds no files".into());
        }
        let wide = incompat & INCOMPAT_64BIT != 0;
        let block_size = 1024u64 << u32_at(&sb, 24);
        let blocks = u32_at(&sb, 4) as u64 | if wide { (u32_at(&sb, 0x150) as u64) << 32 } else { 0 };
        let first_data_block = u32_at(&sb, 20) as u64;
        let blocks_per_group = u32_at(&sb, 32) as u64;
        let inodes_per_group = u32_at(&sb, 40) as u64;
        let inode_size = if u32_at(&sb, 76) == 0 { 128 } else { u16_at(&sb, 88) as u64 };
        let desc_size = if wide { (u16_at(&sb, 254) as u64).max(64) } else { 32 };
        if blocks_per_group == 0 || inodes_per_group == 0 || !(128..=block_size).contains(&inode_size) || desc_size > block_size {
            return Err("the ext superblock's geometry makes no sense".into());
        }
        let ngroups = blocks.saturating_sub(first_data_block).div_ceil(blocks_per_group);
        let mut v = Ext {
            reader,
            part_off,
            block_size,
            blocks,
            first_data_block,
            blocks_per_group,
            inodes_per_group,
            inode_size,
            inodes: u32_at(&sb, 0) as u64,
            filetype: incompat & INCOMPAT_FILETYPE != 0,
            groups: Vec::new(),
            bitmap: None,
            files: HashMap::new(),
        };

        let per_block = block_size / desc_size;
        let meta_bg = incompat & INCOMPAT_META_BG != 0;
        let first_meta_bg = if meta_bg { u32_at(&sb, 0x104) as u64 } else { u64::MAX };
        let sparse = u32_at(&sb, 100) & RO_COMPAT_SPARSE_SUPER != 0;
        let mut blk = vec![0u8; block_size as usize];
        for i in 0..ngroups.div_ceil(per_block) {
            let at = if i < first_meta_bg {
                first_data_block + 1 + i
            } else {
                let g = i * per_block;
                first_data_block + g * blocks_per_group + has_super(g, sparse) as u64
            };
            if v.read_block(at, &mut blk) > 0 && i == 0 {
                return Err("the ext group descriptor table can't be read".into());
            }
            for d in blk.chunks_exact(desc_size as usize) {
                if v.groups.len() as u64 == ngroups {
                    break;
                }
                let hi = |o: usize| if desc_size >= 64 { (u32_at(d, o) as u64) << 32 } else { 0 };
                v.groups.push((u32_at(d, 0) as u64 | hi(0x20), u32_at(d, 8) as u64 | hi(0x28)));
            }
        }
        Ok(v)
    }

    fn read_block(&mut self, b: u64, out: &mut [u8]) -> u64 {
        if b >= self.blocks {
            out.fill(0);
            return out.len() as u64;
        }
        self.reader.read(self.part_off + b * self.block_size, out)
    }

    fn inode(&mut self, n: u32) -> Option<Inode> {
        if n == 0 || n as u64 > self.inodes {
            return None;
        }
        let (g, i) = ((n as u64 - 1) / self.inodes_per_group, (n as u64 - 1) % self.inodes_per_group);
        let table = self.groups.get(g as usize)?.1;
        let mut b = vec![0u8; self.inode_size as usize];
        if table == 0 || self.reader.read(self.part_off + table * self.block_size + i * self.inode_size, &mut b) > 0 {
            return None;
        }
        let extra = if self.inode_size > 128 { u16_at(&b, 128) as u64 } else { 0 };
        Some(Inode {
            mode: u16_at(&b, 0),
            size: u32_at(&b, 4) as u64 | (u32_at(&b, 108) as u64) << 32,
            links: u16_at(&b, 26),
            flags: u32_at(&b, 32),
            mtime: u32_at(&b, 16),
            crtime: (128 + extra >= 148 && self.inode_size >= 148).then(|| u32_at(&b, 144)),
            block: b[40..100].try_into().unwrap(),
        })
    }

    fn map(&mut self, ino: &Inode) -> Map {
        let needed = ino.size.div_ceil(self.block_size);
        let mut m = Map::default();
        if ino.flags & FL_INLINE_DATA != 0 {
            let n = (ino.size as usize).min(60);
            m.inline = Some(ino.block[..n].to_vec());
            // The rest of inline data lives in an extended attribute.
            m.incomplete = ino.size > 60;
        } else if ino.flags & FL_EXTENTS != 0 {
            let mut nodes = 0;
            self.walk_extents(&ino.block, 0, &mut m, &mut nodes);
        } else {
            let ptr = |i: usize| u32_at(&ino.block, i * 4) as u64;
            for i in 0..12u64 {
                self.push_block(&mut m, i, ptr(i as usize));
            }
            let per = self.block_size / 4;
            let mut first = 12u64;
            for (level, slot) in [(1u32, 12usize), (2, 13), (3, 14)] {
                if first >= needed {
                    break;
                }
                self.walk_indirect(ptr(slot), level, first, needed, &mut m);
                first += per.pow(level);
            }
        }
        // Blocks beyond what the size needs aren't data; past the end of
        // the volume they're garbage.
        m.extents.retain(|e| e.0 < needed);
        let blocks = self.blocks;
        if m.extents.iter().any(|e| e.1 + e.2 > blocks) {
            m.incomplete = true;
            m.extents.retain(|e| e.1 + e.2 <= blocks);
        }
        m
    }

    fn push_block(&self, m: &mut Map, logical: u64, phys: u64) {
        if phys == 0 {
            return;
        }
        match m.extents.last_mut() {
            Some((l, p, n)) if *l + *n == logical && *p + *n == phys => *n += 1,
            _ => m.extents.push((logical, phys, 1)),
        }
    }

    fn walk_indirect(&mut self, blk: u64, level: u32, first: u64, needed: u64, m: &mut Map) {
        if blk == 0 {
            return;
        }
        let per = self.block_size / 4;
        let mut buf = vec![0u8; self.block_size as usize];
        if self.read_block(blk, &mut buf) > 0 {
            m.incomplete = true;
            return;
        }
        let span = per.pow(level - 1);
        for i in 0..per {
            let logical = first + i * span;
            if logical >= needed {
                break;
            }
            let p = u32_at(&buf, (i * 4) as usize) as u64;
            if level == 1 {
                self.push_block(m, logical, p);
            } else {
                self.walk_indirect(p, level - 1, logical, needed, m);
            }
        }
    }

    fn walk_extents(&mut self, node: &[u8], depth_seen: u32, m: &mut Map, nodes: &mut usize) {
        *nodes += 1;
        if node.len() < 12 || u16_at(node, 0) != 0xF30A || depth_seen > 5 || *nodes > MAX_EXTENT_NODES {
            m.incomplete |= depth_seen > 0 || u16_at(node, 0) == 0xF30A;
            return;
        }
        let entries = (u16_at(node, 2) as usize).min((node.len() - 12) / 12);
        let depth = u16_at(node, 6);
        for i in 0..entries {
            let e = &node[12 + i * 12..24 + i * 12];
            if depth == 0 {
                let raw_len = u16_at(e, 4) as u64;
                // Above 32768: preallocated but never written, reads as zeros.
                if raw_len > 32768 {
                    continue;
                }
                let phys = (u16_at(e, 6) as u64) << 32 | u32_at(e, 8) as u64;
                m.extents.push((u32_at(e, 0) as u64, phys, raw_len));
            } else {
                let child = (u16_at(e, 8) as u64) << 32 | u32_at(e, 4) as u64;
                let mut buf = vec![0u8; self.block_size as usize];
                if self.read_block(child, &mut buf) > 0 {
                    m.incomplete = true;
                    continue;
                }
                self.walk_extents(&buf, depth_seen + 1, m, nodes);
            }
        }
    }

    /// Reads `out.len()` bytes at byte `pos` of a mapped file. Holes are
    /// zeros; returns how many bytes were unreadable.
    fn read_mapped(&mut self, m: &Map, pos: u64, out: &mut [u8]) -> u64 {
        out.fill(0);
        if let Some(d) = &m.inline {
            let s = (pos as usize).min(d.len());
            let n = (d.len() - s).min(out.len());
            out[..n].copy_from_slice(&d[s..s + n]);
            return 0;
        }
        let bs = self.block_size;
        let end = pos + out.len() as u64;
        let mut bad = 0;
        for &(l, p, n) in &m.extents {
            let (rs, re) = (l * bs, (l + n) * bs);
            let (s, e) = (rs.max(pos), re.min(end));
            if s < e {
                let abs = self.part_off + p * bs + (s - rs);
                bad += self.reader.read(abs, &mut out[(s - pos) as usize..(e - pos) as usize]);
            }
        }
        bad
    }

    fn load_bitmap(&mut self) {
        if self.bitmap.is_some() {
            return;
        }
        let per = self.blocks_per_group.div_ceil(8) as usize;
        let mut bm = vec![0u8; per * self.groups.len()];
        let mut blk = vec![0u8; self.block_size as usize];
        for (g, &(b, _)) in self.groups.clone().iter().enumerate() {
            let dst = &mut bm[g * per..(g + 1) * per];
            if b == 0 || self.read_block(b, &mut blk) > 0 {
                dst.fill(0xFF); // unknown: assume in use
            } else {
                let n = per.min(blk.len());
                dst[..n].copy_from_slice(&blk[..n]);
            }
        }
        self.bitmap = Some(bm);
    }

    fn reused_blocks(&mut self, m: &Map) -> u64 {
        self.load_bitmap();
        let bm = self.bitmap.as_ref().unwrap();
        let per = self.blocks_per_group.div_ceil(8);
        let mut n = 0;
        for &(_, p, len) in &m.extents {
            for b in p..p + len {
                let rel = b.saturating_sub(self.first_data_block);
                let (g, bit) = (rel / self.blocks_per_group, rel % self.blocks_per_group);
                let byte = bm.get((g * per + bit / 8) as usize).copied().unwrap_or(0xFF);
                n += (byte & (1 << (bit % 8)) != 0) as u64;
            }
        }
        n
    }

    fn assess(&mut self, ino: &Inode, m: &Map, deleted: bool) -> Recoverable {
        if ino.is_dir() || ino.size == 0 || (m.inline.is_some() && !m.incomplete) {
            return Recoverable::Full;
        }
        let bs = self.block_size;
        let total: u64 = m.extents.iter().map(|e| e.2 * bs).sum();
        let unreadable: u64 = m.extents.iter().map(|e| self.reader.unreadable_bytes(self.part_off + e.1 * bs, self.part_off + (e.1 + e.2) * bs)).sum();
        let reused = if deleted { self.reused_blocks(m) * bs } else { 0 };
        let lost = (unreadable + reused).min(total);
        if total == 0 || lost >= total {
            Recoverable::None
        } else if lost > 0 || m.incomplete || ino.flags & FL_ENCRYPT != 0 {
            Recoverable::Partial
        } else {
            Recoverable::Full
        }
    }

    fn dir_bytes(&mut self, ino: &Inode) -> Vec<u8> {
        let m = self.map(ino);
        let mut buf = vec![0u8; ino.size.min(MAX_DIR_BYTES) as usize];
        self.read_mapped(&m, 0, &mut buf);
        buf
    }

    /// Live entries and deleted ones left in the slack after a live
    /// entry's name: (inode, name, deleted).
    fn parse_dir(&self, buf: &[u8], hashed: bool) -> Vec<(u32, String, bool)> {
        let bs = self.block_size as usize;
        let name_len = |e: &[u8]| if self.filetype { e[6] as usize } else { u16_at(e, 6) as usize };
        let mut out = Vec::new();
        let mut pos = 0;
        while pos + 8 <= buf.len() {
            let e = &buf[pos..];
            let (ino, rec_len, nl) = (u32_at(e, 0), u16_at(e, 4) as usize, name_len(e));
            if rec_len < 8 || rec_len % 4 != 0 || pos + rec_len > buf.len() || pos / bs != (pos + rec_len - 1) / bs {
                pos = (pos / bs + 1) * bs; // damaged block: try the next one
                continue;
            }
            let name = &e[8..(8 + nl).min(rec_len)];
            if ino != 0 && nl > 0 && 8 + nl <= rec_len {
                out.push((ino, String::from_utf8_lossy(name).into_owned(), false));
            }
            // A hashed directory hides its index in ".." and in empty
            // entries; that isn't slack.
            let index_block = hashed && ((ino == 0 && nl == 0) || name == b"..");
            let used = if ino == 0 { 0 } else { (8 + nl + 3) & !3 };
            if !index_block {
                let mut q = pos + used;
                while q + 12 <= pos + rec_len {
                    match self.deleted_entry(&buf[q..pos + rec_len]) {
                        Some((i, n, len)) => {
                            out.push((i, n, true));
                            q += len;
                        }
                        None => q += 4,
                    }
                }
            }
            pos += rec_len;
        }
        out.retain(|(_, n, _)| n != "." && n != "..");
        out
    }

    fn deleted_entry(&self, b: &[u8]) -> Option<(u32, String, usize)> {
        let (ino, rec_len) = (u32_at(b, 0), u16_at(b, 4) as usize);
        let nl = if self.filetype { b[6] as usize } else { u16_at(b, 6) as usize };
        let ok = ino != 0
            && (ino as u64) <= self.inodes
            && rec_len >= 8 + nl
            && rec_len % 4 == 0
            && nl > 0
            && 8 + nl <= b.len()
            && (!self.filetype || (1..=7).contains(&b[7]))
            && b[8..8 + nl].iter().all(|&c| c != 0 && c != b'/');
        ok.then(|| (ino, String::from_utf8_lossy(&b[8..8 + nl]).into_owned(), (8 + nl + 3) & !3))
    }
}

/// Whether group `g` holds a superblock backup (and so its descriptor
/// block sits one block in, under meta_bg).
fn has_super(g: u64, sparse: bool) -> bool {
    if !sparse || g <= 1 {
        return true;
    }
    [3u64, 5, 7].iter().any(|&b| {
        let mut x = g;
        while x.is_multiple_of(b) {
            x /= b;
        }
        x == 1
    })
}

impl Volume for Ext<'_, '_> {
    /// Walks the directory tree breadth-first from the root inode;
    /// `progress` counts directories (the total grows as more are found).
    fn index(&mut self, progress: &mut dyn FnMut(u64, u64)) -> Index {
        let mut entries = HashMap::new();
        entries.insert(
            ROOT,
            IndexEntry {
                seq: 0,
                in_use: true,
                is_dir: true,
                parent_ref: ROOT,
                name: String::new(),
                size: 0,
                created: 0,
                modified: 0,
                recoverable: Recoverable::Full,
            },
        );
        let mut queue = VecDeque::from([(ROOT, ROOT_INODE, false)]);
        let mut seen = HashSet::from([ROOT_INODE]);
        let mut next_id = FIRST_USER_RECORD;
        let mut done = 0;
        while let Some((dir_id, dir_ino, dir_deleted)) = queue.pop_front() {
            let Some(d) = self.inode(dir_ino) else { continue };
            let bytes = self.dir_bytes(&d);
            for (n, name, slack) in self.parse_dir(&bytes, d.flags & FL_INDEX != 0) {
                let Some(ino) = self.inode(n) else { continue };
                let deleted = slack || dir_deleted;
                // A deleted name whose inode is in use again is a rename
                // leftover or points at someone else's file now.
                if slack && ino.links != 0 {
                    continue;
                }
                let kind = ino.mode & S_IFMT;
                if kind != S_IFDIR && kind != S_IFREG {
                    continue; // symlinks, devices, sockets, fifos
                }
                let m = self.map(&ino);
                let recoverable = self.assess(&ino, &m, deleted);
                let id = next_id;
                next_id += 1;
                if ino.is_dir() && seen.insert(n) {
                    queue.push_back((id, n, deleted));
                }
                entries.insert(
                    id,
                    IndexEntry {
                        seq: 0,
                        in_use: !deleted,
                        is_dir: ino.is_dir(),
                        parent_ref: dir_id,
                        name,
                        size: if ino.is_dir() { 0 } else { ino.size },
                        created: ino.crtime.map_or(0, |t| unix_to_filetime(t as i64)),
                        modified: unix_to_filetime(ino.mtime as i64),
                        recoverable,
                    },
                );
                self.files.insert(id, (n, deleted));
            }
            done += 1;
            progress(done, done + queue.len() as u64);
        }
        Index::from_entries(entries)
    }

    fn copy_file(&mut self, _index: &Index, id: u64, out: &mut dyn Write) -> Result<(u64, Vec<String>), String> {
        let &(n, deleted) = self.files.get(&id).ok_or("this file isn't in the volume's index")?;
        let ino = self.inode(n).ok_or("its inode can no longer be read")?;
        let m = self.map(&ino);
        let mut notes = Vec::new();
        if m.extents.is_empty() && m.inline.is_none() && ino.size > 0 {
            notes.push("Its block pointers were wiped when it was deleted (ext3/ext4 do this), so its contents can't be located; try carving instead.".to_string());
            return Ok((0, notes));
        }
        let reused = if deleted { self.reused_blocks(&m) } else { 0 };
        let mut buf = vec![0u8; COPY_CHUNK as usize];
        let (mut pos, mut bad) = (0u64, 0u64);
        while pos < ino.size {
            let n = (ino.size - pos).min(COPY_CHUNK) as usize;
            bad += self.read_mapped(&m, pos, &mut buf[..n]);
            out.write_all(&buf[..n]).map_err(|e| format!("writing failed: {e}"))?;
            pos += n as u64;
        }
        if bad > 0 {
            notes.push(unreadable_note(bad));
        }
        if m.incomplete {
            notes.push("Part of this file's block map is missing or damaged.".to_string());
        }
        if reused > 0 {
            notes.push(REUSED_NOTE.to_string());
        }
        if ino.flags & FL_ENCRYPT != 0 {
            notes.push("This file is encrypted by ext4 (fscrypt); it was copied still encrypted.".to_string());
        }
        Ok((ino.size, notes))
    }
}
