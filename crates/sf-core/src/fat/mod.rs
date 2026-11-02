//! FAT12/16/32 and exFAT file listing and recovery, including deleted
//! files, presented through the same [`Index`] and [`Volume`] as NTFS.
//!
//! Live files follow their cluster chain. FAT zeroes a deleted file's chain,
//! so a deleted file is read as one contiguous run from its first cluster
//! (what every FAT undelete tool does); clusters that are allocated again
//! are counted as reused. Unreadable FAT or bitmap sectors read as
//! "allocated", so damage never makes a file look better than it is.

mod dir;

#[cfg(test)]
mod tests;

use std::collections::{HashMap, HashSet, VecDeque};
use std::io::Write;

use crate::ntfs::{unreadable_note, Index, IndexEntry, Recoverable, Volume, FIRST_USER_RECORD, REUSED_NOTE, ROOT};
use crate::reader::SafeReader;
use dir::RawEntry;

const FREE: u32 = 0;
const BAD: u32 = u32::MAX - 1;
const EOC: u32 = u32::MAX;
const COPY_CHUNK: u64 = 4 * 1024 * 1024;
/// Live directories bigger than this are corrupt chains, not directories.
const MAX_DIR_BYTES: u64 = 64 * 1024 * 1024;
/// A deleted FAT directory has no size; read at most this many free
/// clusters after its first one.
const DELETED_DIR_CLUSTERS: u64 = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FatKind {
    Fat12,
    Fat16,
    Fat32,
    ExFat,
}

/// Where a file's (or directory's) data starts and how to follow it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Loc {
    first: u32,
    size: u64,
    /// Bytes past this read as zeros (exFAT's valid data length).
    valid: u64,
    /// No chain to follow: exFAT's NoFatChain flag.
    contiguous: bool,
    deleted: bool,
}

#[derive(Debug, Clone, Copy)]
enum DirSrc {
    /// FAT12/16's fixed root directory region: (offset in volume, bytes).
    FixedRoot(u64, u64),
    /// A live FAT directory: follow the chain.
    Chain(u32),
    /// A deleted FAT directory: free clusters from here on.
    Free(u32),
    /// An exFAT directory, whose size is recorded like a file's.
    Loc(Loc),
}

pub struct Fat<'r, 'a> {
    reader: &'r mut SafeReader<'a>,
    part_off: u64,
    pub kind: FatKind,
    cluster_size: u64,
    /// Volume offset of cluster 2.
    heap_off: u64,
    /// Highest cluster number + 1.
    cluster_end: u32,
    root: DirSrc,
    /// Next cluster for every cluster, normalized to FREE/BAD/EOC.
    table: Vec<u32>,
    /// exFAT's allocation bitmap (bit 0 = cluster 2).
    bitmap: Option<Vec<u8>>,
    files: HashMap<u64, Loc>,
}

fn u16_at(b: &[u8], o: usize) -> u16 {
    u16::from_le_bytes([b[o], b[o + 1]])
}
fn u32_at(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes(b[o..o + 4].try_into().unwrap())
}
fn u64_at(b: &[u8], o: usize) -> u64 {
    u64::from_le_bytes(b[o..o + 8].try_into().unwrap())
}

impl<'r, 'a> Fat<'r, 'a> {
    pub fn open(reader: &'r mut SafeReader<'a>, part_off: u64) -> Result<Self, String> {
        let mut bs = vec![0u8; 512];
        if reader.read(part_off, &mut bs) > 0 {
            return Err("the FAT boot sector can't be read".into());
        }
        let mut v = Fat {
            reader,
            part_off,
            kind: FatKind::Fat12,
            cluster_size: 0,
            heap_off: 0,
            cluster_end: 0,
            root: DirSrc::Chain(0),
            table: Vec::new(),
            bitmap: None,
            files: HashMap::new(),
        };
        if &bs[3..11] == b"EXFAT   " {
            v.open_exfat(&bs)?;
        } else {
            v.open_fat(&bs)?;
        }
        Ok(v)
    }

    fn open_fat(&mut self, b: &[u8]) -> Result<(), String> {
        let bps = u16_at(b, 11) as u64;
        let spc = b[13] as u64;
        let reserved = u16_at(b, 14) as u64;
        let nfats = b[16] as u64;
        let root_ents = u16_at(b, 17) as u64;
        let total = match u16_at(b, 19) {
            0 => u32_at(b, 32) as u64,
            n => n as u64,
        };
        let fat_sectors = match u16_at(b, 22) {
            0 => u32_at(b, 36) as u64,
            n => n as u64,
        };
        if !matches!(bps, 512 | 1024 | 2048 | 4096) || !spc.is_power_of_two() || reserved == 0 || !(1..=4).contains(&nfats) || fat_sectors == 0 {
            return Err("no valid FAT boot sector at this partition offset".into());
        }
        let root_sectors = (root_ents * 32).div_ceil(bps);
        let first_data = reserved + nfats * fat_sectors + root_sectors;
        let count = total.checked_sub(first_data).ok_or("the FAT boot sector's sizes don't add up")? / spc;
        self.kind = match count {
            0..4085 => FatKind::Fat12,
            4085..65525 => FatKind::Fat16,
            _ => FatKind::Fat32,
        };
        self.cluster_size = bps * spc;
        self.heap_off = first_data * bps;
        self.cluster_end = (count + 2).min(0x0FFF_FFF7) as u32;
        self.root = match self.kind {
            FatKind::Fat32 => DirSrc::Chain(u32_at(b, 44)),
            _ => DirSrc::FixedRoot((reserved + nfats * fat_sectors) * bps, root_ents * 32),
        };
        let copies: Vec<u64> = (0..nfats).map(|i| (reserved + i * fat_sectors) * bps).collect();
        self.load_table(&copies, fat_sectors * bps);
        Ok(())
    }

    fn open_exfat(&mut self, b: &[u8]) -> Result<(), String> {
        let (bps_shift, spc_shift) = (b[0x6C] as u32, b[0x6D] as u32);
        if !(9..=12).contains(&bps_shift) || bps_shift + spc_shift > 25 || !(1..=2).contains(&b[0x6E]) {
            return Err("no valid exFAT boot sector at this partition offset".into());
        }
        let bps = 1u64 << bps_shift;
        let (fat_off, fat_len) = (u32_at(b, 0x50) as u64 * bps, u32_at(b, 0x54) as u64 * bps);
        self.kind = FatKind::ExFat;
        self.cluster_size = bps << spc_shift;
        self.heap_off = u32_at(b, 0x58) as u64 * bps;
        self.cluster_end = u32_at(b, 0x5C).saturating_add(2).min(0xFFFF_FFF6);
        self.root = DirSrc::Chain(u32_at(b, 0x60));
        let copies: Vec<u64> = (0..b[0x6E] as u64).map(|i| fat_off + i * fat_len).collect();
        self.load_table(&copies, fat_len);

        // The allocation bitmap is described by an entry in the root.
        let root = self.dir_bytes(self.root);
        let (first, len) = dir::exfat_bitmap(&root).ok_or("the exFAT root directory has no allocation bitmap entry")?;
        let loc = Loc { first, size: len, valid: len, contiguous: false, deleted: true };
        let (runs, _) = self.runs(&loc);
        let mut bm = vec![0u8; len as usize];
        self.read_runs(&runs, 0, &mut bm, true);
        self.bitmap = Some(bm);
        Ok(())
    }

    /// Reads and decodes the FAT, taking each unreadable sector from the
    /// next copy that has it readable.
    fn load_table(&mut self, copies: &[u64], fat_bytes: u64) {
        let entries = self.cluster_end as u64;
        let need = match self.kind {
            FatKind::Fat12 => (entries * 3).div_ceil(2),
            FatKind::Fat16 => entries * 2,
            FatKind::Fat32 | FatKind::ExFat => entries * 4,
        }
        .min(fat_bytes) as usize;
        let mut raw = vec![0u8; need];
        let base = self.part_off + copies[0];
        if self.reader.read(base, &mut raw) > 0 {
            let piece = self.reader.sector_size().max(512) as usize;
            for (i, chunk) in raw.chunks_mut(piece).enumerate() {
                let at = (i * piece) as u64;
                if !self.reader.is_unreadable(base + at, base + at + chunk.len() as u64) {
                    continue;
                }
                let alt = copies[1..].iter().find_map(|&c| {
                    let mut tmp = vec![0u8; chunk.len()];
                    (self.reader.read(self.part_off + c + at, &mut tmp) == 0).then_some(tmp)
                });
                match alt {
                    Some(tmp) => chunk.copy_from_slice(&tmp),
                    None => chunk.fill(0xFF), // reads as end-of-chain: allocated
                }
            }
        }
        let (eoc, bad) = match self.kind {
            FatKind::Fat12 => (0xFF8, 0xFF7),
            FatKind::Fat16 => (0xFFF8, 0xFFF7),
            FatKind::Fat32 => (0x0FFF_FFF8, 0x0FFF_FFF7),
            FatKind::ExFat => (0xFFFF_FFF8, 0xFFFF_FFF7),
        };
        self.table = (0..entries as usize)
            .map(|n| {
                let v = match self.kind {
                    FatKind::Fat12 => {
                        let o = n + n / 2;
                        match raw.get(o..o + 2) {
                            Some(w) => {
                                let w = u16_at(w, 0) as u32;
                                if n % 2 == 1 { w >> 4 } else { w & 0xFFF }
                            }
                            None => eoc,
                        }
                    }
                    FatKind::Fat16 => raw.get(n * 2..n * 2 + 2).map_or(eoc, |w| u16_at(w, 0) as u32),
                    FatKind::Fat32 => raw.get(n * 4..n * 4 + 4).map_or(eoc, |w| u32_at(w, 0) & 0x0FFF_FFFF),
                    FatKind::ExFat => raw.get(n * 4..n * 4 + 4).map_or(eoc, |w| u32_at(w, 0)),
                };
                if v >= eoc {
                    EOC
                } else if v == bad {
                    BAD
                } else {
                    v
                }
            })
            .collect();
    }

    fn in_heap(&self, c: u32) -> bool {
        (2..self.cluster_end).contains(&c)
    }

    /// Whether cluster `c` belongs to something now (or can't be told).
    fn allocated(&self, c: u32) -> bool {
        if !self.in_heap(c) {
            return true;
        }
        match &self.bitmap {
            Some(bm) => bm.get(((c - 2) / 8) as usize).is_none_or(|b| b & (1 << ((c - 2) % 8)) != 0),
            None => self.table[c as usize] != FREE,
        }
    }

    /// Follows the chain from `first` for at most `max` clusters. Returns
    /// (first cluster, length) runs and the number of clusters found.
    fn chain(&self, first: u32, max: u64) -> (Vec<(u32, u32)>, u64) {
        let mut runs: Vec<(u32, u32)> = Vec::new();
        let mut n = 0u64;
        let mut c = first;
        // Never more steps than there are clusters: a looped chain ends here.
        while self.in_heap(c) && n < max && n < self.cluster_end as u64 {
            match runs.last_mut() {
                Some((s, l)) if *s + *l == c => *l += 1,
                _ => runs.push((c, 1)),
            }
            n += 1;
            match self.table[c as usize] {
                next if self.in_heap(next) => c = next,
                _ => break,
            }
        }
        (runs, n)
    }

    /// The clusters holding `loc`'s data, and whether all of them were found.
    fn runs(&self, loc: &Loc) -> (Vec<(u32, u32)>, bool) {
        let needed = loc.size.div_ceil(self.cluster_size);
        if needed == 0 {
            return (Vec::new(), true);
        }
        if !self.in_heap(loc.first) {
            return (Vec::new(), false);
        }
        if !loc.contiguous {
            let (runs, n) = self.chain(loc.first, needed);
            // A deleted file's chain is normally gone; fall back to one run.
            if n >= needed || !loc.deleted {
                return (runs, n >= needed);
            }
        }
        let avail = needed.min((self.cluster_end - loc.first) as u64);
        (vec![(loc.first, avail as u32)], avail == needed)
    }

    /// Reads `out.len()` bytes at byte `pos` of the data in `runs`. Returns
    /// the bytes that were unreadable or beyond the runs (zeros, or 0xFF
    /// with `ff_bad`, for bitmaps where "unknown" must mean "allocated").
    fn read_runs(&mut self, runs: &[(u32, u32)], pos: u64, out: &mut [u8], ff_bad: bool) -> u64 {
        let fill = if ff_bad { 0xFF } else { 0 };
        out.fill(fill);
        let cs = self.cluster_size;
        let end = pos + out.len() as u64;
        let (mut covered, mut bad, mut vpos) = (0u64, 0u64, 0u64);
        for &(c, len) in runs {
            let (rs, re) = (vpos, vpos + len as u64 * cs);
            vpos = re;
            let (s, e) = (rs.max(pos), re.min(end));
            if s >= e {
                continue;
            }
            covered += e - s;
            let abs = self.part_off + self.heap_off + (c as u64 - 2) * cs + (s - rs);
            let dst = &mut out[(s - pos) as usize..(e - pos) as usize];
            let b = self.reader.read(abs, dst);
            bad += b;
            if b > 0 && ff_bad {
                let piece = self.reader.sector_size().max(512);
                for (i, ch) in dst.chunks_mut(piece as usize).enumerate() {
                    let a = abs + i as u64 * piece;
                    if self.reader.is_unreadable(a, a + ch.len() as u64) {
                        ch.fill(0xFF);
                    }
                }
            }
        }
        bad + (out.len() as u64 - covered)
    }

    /// (located bytes, unreadable bytes, reused bytes) of `loc`'s data.
    fn damage(&self, runs: &[(u32, u32)], loc: &Loc) -> (u64, u64, u64) {
        let cs = self.cluster_size;
        let (mut total, mut unreadable, mut reused) = (0, 0, 0);
        for &(c, len) in runs {
            total += len as u64 * cs;
            let abs = self.part_off + self.heap_off + (c as u64 - 2) * cs;
            unreadable += self.reader.unreadable_bytes(abs, abs + len as u64 * cs);
            if loc.deleted {
                reused += (c..c + len).filter(|&x| self.allocated(x)).count() as u64 * cs;
            }
        }
        (total, unreadable, reused)
    }

    fn assess(&self, loc: &Loc, is_dir: bool) -> Recoverable {
        if is_dir || loc.size == 0 {
            return Recoverable::Full;
        }
        let (runs, complete) = self.runs(loc);
        let (total, unreadable, reused) = self.damage(&runs, loc);
        let lost = (unreadable + reused).min(total);
        if total == 0 || lost >= total {
            Recoverable::None
        } else if lost > 0 || !complete {
            Recoverable::Partial
        } else {
            Recoverable::Full
        }
    }

    fn dir_bytes(&mut self, src: DirSrc) -> Vec<u8> {
        let max = MAX_DIR_BYTES / self.cluster_size;
        let (runs, n) = match src {
            DirSrc::FixedRoot(off, len) => {
                let mut buf = vec![0u8; len as usize];
                self.reader.read(self.part_off + off, &mut buf);
                return buf;
            }
            DirSrc::Chain(first) => self.chain(first, max),
            DirSrc::Free(first) => {
                if !self.in_heap(first) || self.allocated(first) {
                    return Vec::new();
                }
                let n = (first..self.cluster_end).take(DELETED_DIR_CLUSTERS as usize).take_while(|&c| !self.allocated(c)).count();
                (vec![(first, n as u32)], n as u64)
            }
            DirSrc::Loc(loc) => {
                if loc.deleted && self.allocated(loc.first) {
                    return Vec::new(); // reused: whatever is there now isn't this directory
                }
                let loc = Loc { size: loc.size.min(MAX_DIR_BYTES), ..loc };
                let (runs, _) = self.runs(&loc);
                let n = runs.iter().map(|r| r.1 as u64).sum();
                (runs, n)
            }
        };
        let mut buf = vec![0u8; (n * self.cluster_size) as usize];
        self.read_runs(&runs, 0, &mut buf, false);
        buf
    }
}

impl Volume for Fat<'_, '_> {
    /// Walks the directory tree breadth-first from the root. `progress`
    /// counts directories (the total grows as more are found).
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
        let mut queue = VecDeque::from([(ROOT, self.root, false)]);
        let mut seen: HashSet<u32> = HashSet::new();
        if let DirSrc::Chain(c) = self.root {
            seen.insert(c);
        }
        let mut next_id = FIRST_USER_RECORD;
        let mut done = 0;
        while let Some((dir_id, src, parent_deleted)) = queue.pop_front() {
            let bytes = self.dir_bytes(src);
            let raw: Vec<RawEntry> = match self.kind {
                FatKind::ExFat => dir::parse_exfat(&bytes),
                // A deleted directory's clusters past its end hold anything;
                // stop at the first entry that doesn't look like one.
                _ => dir::parse_fat(&bytes, self.kind == FatKind::Fat32, matches!(src, DirSrc::Free(_))),
            };
            for r in raw {
                let deleted = r.deleted || parent_deleted;
                let mut loc = r.loc;
                loc.deleted = deleted;
                // FAT keeps no chain for a deleted file; exFAT may still have one.
                loc.contiguous |= deleted && self.kind != FatKind::ExFat;
                if !self.in_heap(loc.first) && (r.is_dir || loc.size > 0) && loc.first != 0 {
                    continue; // points outside the volume: garbage
                }
                let id = next_id;
                next_id += 1;
                let recoverable = self.assess(&loc, r.is_dir);
                if r.is_dir && self.in_heap(loc.first) && seen.insert(loc.first) {
                    let src = match (self.kind, deleted) {
                        (FatKind::ExFat, _) => DirSrc::Loc(loc),
                        (_, false) => DirSrc::Chain(loc.first),
                        (_, true) => DirSrc::Free(loc.first),
                    };
                    queue.push_back((id, src, deleted));
                }
                entries.insert(
                    id,
                    IndexEntry {
                        seq: 0,
                        in_use: !deleted,
                        is_dir: r.is_dir,
                        parent_ref: dir_id,
                        name: r.name,
                        size: if r.is_dir { 0 } else { loc.size },
                        created: r.created,
                        modified: r.modified,
                        recoverable,
                    },
                );
                self.files.insert(id, loc);
            }
            done += 1;
            progress(done, done + queue.len() as u64);
        }
        Index::from_entries(entries)
    }

    fn copy_file(&mut self, _index: &Index, id: u64, out: &mut dyn Write) -> Result<(u64, Vec<String>), String> {
        let loc = *self.files.get(&id).ok_or("this file isn't in the volume's index")?;
        let (runs, complete) = self.runs(&loc);
        let (_, _, reused) = self.damage(&runs, &loc);
        let mut buf = vec![0u8; COPY_CHUNK as usize];
        let (mut pos, mut bad) = (0u64, 0u64);
        while pos < loc.size {
            let n = (loc.size - pos).min(COPY_CHUNK) as usize;
            let chunk = &mut buf[..n];
            let valid = (loc.valid.saturating_sub(pos) as usize).min(n);
            bad += self.read_runs(&runs, pos, &mut chunk[..valid], false);
            chunk[valid..].fill(0);
            out.write_all(chunk).map_err(|e| format!("writing failed: {e}"))?;
            pos += n as u64;
        }
        let mut notes = Vec::new();
        if bad > 0 {
            notes.push(unreadable_note(bad));
        }
        if !complete {
            notes.push("Part of this file's cluster chain is missing or runs off the volume.".to_string());
        }
        if reused > 0 {
            notes.push(REUSED_NOTE.to_string());
        }
        Ok((loc.size, notes))
    }
}
