//! An NTFS volume opened through a [`SafeReader`]: boot sector, the MFT's
//! own (possibly fragmented) runlist, record lookup with `$ATTRIBUTE_LIST`
//! resolution, the cluster bitmap, and streaming a file's data out.

use std::io::{self, Write};

use super::record::*;
use super::runlist::Run;
use crate::reader::SafeReader;

const COPY_CHUNK: u64 = 4 * 1024 * 1024;

#[derive(Debug, Clone, Copy)]
pub struct Boot {
    pub cluster_size: u64,
    pub record_size: u64,
    pub mft_lcn: u64,
    pub mftmirr_lcn: u64,
    pub total_bytes: u64,
}

pub fn parse_boot(b: &[u8]) -> Option<Boot> {
    if b.len() < 512 || &b[3..11] != b"NTFS    " {
        return None;
    }
    let bps = u16_at(b, 0x0B) as u64;
    if !matches!(bps, 512 | 1024 | 2048 | 4096) {
        return None;
    }
    let spc = match b[0x0D] {
        n @ 1..=0x80 => n as u64,
        n => 1u64.checked_shl(256 - n as u32)?, // large clusters: 2^(256-n)
    };
    let cluster_size = bps * spc;
    let record_size = match b[0x40] as i8 {
        n if n > 0 => n as u64 * cluster_size,
        n => 1u64.checked_shl((-(n as i32)) as u32)?,
    };
    if !spc.is_power_of_two() || cluster_size > 2 * 1024 * 1024 || !(512..=65536).contains(&record_size) || record_size % 512 != 0 {
        return None;
    }
    let total_bytes = u64_at(b, 0x28).checked_mul(bps)?;
    let mft_lcn = u64_at(b, 0x30);
    if mft_lcn.checked_mul(cluster_size)? >= total_bytes {
        return None;
    }
    Some(Boot { cluster_size, record_size, mft_lcn, mftmirr_lcn: u64_at(b, 0x38), total_bytes })
}

/// The unnamed `$DATA` stream of a file, assembled from all its pieces.
#[derive(Debug, Clone, Default)]
pub struct DataStream {
    pub size: u64,
    pub init_size: u64,
    pub resident: Option<Vec<u8>>,
    pub runs: Vec<Run>,
    pub compressed: bool,
    pub encrypted: bool,
    /// Pieces of the runlist are missing (a lost extension record, a gap
    /// between fragments): those clusters can't be located.
    pub incomplete: bool,
}

#[derive(Debug, Clone)]
pub struct FileEntry {
    pub id: u64,
    pub seq: u16,
    pub in_use: bool,
    pub is_dir: bool,
    pub name: Option<FileName>,
    pub created: u64,
    pub modified: u64,
    pub data: Option<DataStream>,
}

pub struct Ntfs<'r, 'a> {
    pub(crate) reader: &'r mut SafeReader<'a>,
    part_off: u64,
    pub boot: Boot,
    mft: DataStream,
}

impl<'r, 'a> Ntfs<'r, 'a> {
    pub fn open(reader: &'r mut SafeReader<'a>, part_off: u64) -> Result<Self, String> {
        let mut bs = vec![0u8; 4096];
        if reader.read(part_off, &mut bs[..512]) > 0 {
            return Err("the NTFS boot sector can't be read".into());
        }
        reader.read(part_off, &mut bs);
        let boot = parse_boot(&bs).ok_or("no valid NTFS boot sector at this partition offset")?;
        let mut v = Self { reader, part_off, boot, mft: DataStream::default() };

        // Bootstrap: record 0 describes the MFT itself. The mirror holds a
        // copy of the first records in case the primary is damaged.
        let rec0 = [boot.mft_lcn, boot.mftmirr_lcn]
            .into_iter()
            .find_map(|lcn| v.read_physical_record(lcn * boot.cluster_size).and_then(|r| parse_record(&r)))
            .ok_or("the MFT's first record ($MFT) is unreadable in both the MFT and its mirror")?;
        v.mft = v.assemble_data(0, &rec0, false);
        if rec0.attrs.iter().any(|a| a.typ == ATTR_ATTRIBUTE_LIST) {
            // A very fragmented MFT continues its runlist in extension
            // records, which are found through the part located so far.
            if let Some(ds) = v.entry_from(0, rec0).data {
                v.mft = ds;
            }
        }
        if v.mft.runs.is_empty() || v.mft.size < 16 * boot.record_size {
            return Err("the $MFT record has no usable data runs".into());
        }
        if v.mft.incomplete {
            // A lost extension record can hide part of a heavily fragmented
            // MFT; carry on with what can be located.
            v.mft.size = v.mft.runs.iter().map(|r| r.vcn + r.len).max().unwrap_or(0) * boot.cluster_size;
        }
        Ok(v)
    }

    pub fn record_count(&self) -> u64 {
        self.mft.size / self.boot.record_size
    }

    fn read_physical_record(&mut self, vol_off: u64) -> Option<Vec<u8>> {
        let mut r = vec![0u8; self.boot.record_size as usize];
        self.reader.read(self.part_off + vol_off, &mut r);
        apply_fixups(&mut r).then_some(r)
    }

    /// Reads `out.len()` bytes at byte `pos` of a non-resident stream.
    /// Sparse runs read as zeros; clusters that can't be located (beyond the
    /// runlist) or read are zeros and counted in the return value.
    pub fn read_runs(&mut self, runs: &[Run], pos: u64, out: &mut [u8]) -> u64 {
        out.fill(0);
        let cs = self.boot.cluster_size;
        let end = pos + out.len() as u64;
        let mut covered = 0u64;
        let mut bad = 0u64;
        for r in runs {
            let (rs, re) = (r.vcn * cs, (r.vcn + r.len) * cs);
            let (s, e) = (rs.max(pos), re.min(end));
            if s >= e {
                continue;
            }
            covered += e - s;
            if let Some(lcn) = r.lcn {
                let dst = &mut out[(s - pos) as usize..(e - pos) as usize];
                bad += self.reader.read(self.part_off + lcn * cs + (s - rs), dst);
            }
        }
        bad + (out.len() as u64 - covered)
    }

    /// Record `n`, fixed up and parsed; `None` if unreadable, torn or not a
    /// record.
    pub fn record(&mut self, n: u64) -> Option<Record> {
        if n >= self.record_count() {
            return None;
        }
        let rs = self.boot.record_size;
        let mut r = vec![0u8; rs as usize];
        let runs = std::mem::take(&mut self.mft.runs);
        let bad = self.read_runs(&runs, n * rs, &mut r);
        self.mft.runs = runs;
        if bad > 0 || !apply_fixups(&mut r) {
            return None;
        }
        parse_record(&r)
    }

    /// Reads records `[first, first + count)` in one sweep (much faster
    /// than one read per record on a big MFT) and calls `each` with every
    /// one that parses.
    pub fn for_each_record(&mut self, first: u64, count: u64, mut each: impl FnMut(u64, Record)) {
        let rs = self.boot.record_size;
        let mut buf = vec![0u8; (count * rs) as usize];
        let runs = std::mem::take(&mut self.mft.runs);
        self.read_runs(&runs, first * rs, &mut buf);
        self.mft.runs = runs;
        for (i, rec) in buf.chunks_exact_mut(rs as usize).enumerate() {
            if apply_fixups(rec) {
                if let Some(r) = parse_record(rec) {
                    each(first + i as u64, r);
                }
            }
        }
    }

    /// The full entry for base record `id`, following `$ATTRIBUTE_LIST`
    /// into extension records.
    pub fn file(&mut self, id: u64) -> Option<FileEntry> {
        let rec = self.record(id)?;
        Some(self.entry_from(id, rec))
    }

    pub fn entry_from(&mut self, id: u64, rec: Record) -> FileEntry {
        let mut attrs = rec.attrs.clone();
        let mut list_incomplete = false;
        if let Some(list) = rec.attrs.iter().find(|a| a.typ == ATTR_ATTRIBUTE_LIST) {
            let (extra, lost) = self.resolve_attr_list(id, rec.seq, list);
            attrs.extend(extra);
            list_incomplete = lost;
        }

        let name = attrs
            .iter()
            .filter(|a| a.typ == ATTR_FILE_NAME)
            .filter_map(|a| match &a.body {
                AttrBody::Resident(v) => parse_file_name(v),
                _ => None,
            })
            .min_by_key(|f| namespace_rank(f.namespace));
        let si = attrs.iter().find_map(|a| match (&a.body, a.typ) {
            (AttrBody::Resident(v), ATTR_STANDARD_INFORMATION) => parse_std_info(v),
            _ => None,
        });
        let (created, modified) = si.or(name.as_ref().map(|n| (n.created, n.modified))).unwrap_or((0, 0));
        let full = Record { attrs, ..rec };
        let data = (!full.is_dir || full.attrs.iter().any(|a| a.typ == ATTR_DATA && !a.named))
            .then(|| self.assemble_data(id, &full, list_incomplete))
            .filter(|d| !full.is_dir || d.size > 0);
        FileEntry {
            id,
            seq: full.seq,
            in_use: full.in_use,
            is_dir: full.is_dir,
            name,
            created,
            modified,
            data,
        }
    }

    /// Attributes listed in `$ATTRIBUTE_LIST` that live in other records.
    /// Returns them and whether any couldn't be found.
    fn resolve_attr_list(&mut self, id: u64, seq: u16, list: &Attr) -> (Vec<Attr>, bool) {
        let bytes = match &list.body {
            AttrBody::Resident(v) => v.clone(),
            AttrBody::NonResident(nr) => {
                let mut v = vec![0u8; nr.data_size.min(16 * 1024 * 1024) as usize];
                let runs = nr.runs.clone();
                if self.read_runs(&runs, 0, &mut v) > 0 {
                    return (Vec::new(), true);
                }
                v
            }
        };
        let mut out = Vec::new();
        let mut lost = false;
        let mut cache: Vec<(u64, Option<Record>)> = Vec::new();
        for e in parse_attr_list(&bytes) {
            let n = record_number(e.record_ref);
            if n == id {
                continue;
            }
            if !cache.iter().any(|(k, _)| *k == n) {
                // An extension record is only ours if it points back at us;
                // after deletion it may have been reused by another file.
                let r = self
                    .record(n)
                    .filter(|r| record_number(r.base_ref) == id && ref_seq(r.base_ref) == seq && ref_seq(e.record_ref) == r.seq);
                cache.push((n, r));
            }
            let found = cache
                .iter()
                .find(|(k, _)| *k == n)
                .and_then(|(_, r)| r.as_ref())
                .and_then(|r| r.attrs.iter().find(|a| a.typ == e.typ && a.id == e.id));
            match found {
                Some(a) => out.push(a.clone()),
                None => lost = true,
            }
        }
        (out, lost)
    }

    fn assemble_data(&mut self, _id: u64, rec: &Record, list_incomplete: bool) -> DataStream {
        let mut frags: Vec<(&Attr, &NonResident)> = Vec::new();
        for a in rec.attrs.iter().filter(|a| a.typ == ATTR_DATA && !a.named) {
            match &a.body {
                AttrBody::Resident(v) => {
                    return DataStream {
                        size: v.len() as u64,
                        init_size: v.len() as u64,
                        resident: Some(v.clone()),
                        compressed: a.flags & FLAG_COMPRESSED != 0,
                        encrypted: a.flags & FLAG_ENCRYPTED != 0,
                        ..Default::default()
                    };
                }
                AttrBody::NonResident(nr) => frags.push((a, nr)),
            }
        }
        frags.sort_by_key(|(_, nr)| nr.start_vcn);
        let Some((first_attr, first)) = frags.first().copied() else {
            return DataStream { incomplete: list_incomplete, ..Default::default() };
        };
        let mut ds = DataStream {
            size: first.data_size,
            init_size: first.init_size.min(first.data_size),
            // Sparse files also carry a compression unit; only the flag means compressed.
            compressed: first_attr.flags & FLAG_COMPRESSED != 0,
            encrypted: first_attr.flags & FLAG_ENCRYPTED != 0,
            incomplete: list_incomplete || first.start_vcn != 0,
            ..Default::default()
        };
        let mut next_vcn = 0u64;
        for (_, nr) in &frags {
            if nr.start_vcn != next_vcn {
                ds.incomplete = true;
            }
            ds.runs.extend(nr.runs.iter().copied());
            next_vcn = nr.last_vcn.wrapping_add(1);
        }
        let needed = ds.init_size.div_ceil(self.boot.cluster_size);
        let mapped: u64 = ds.runs.iter().filter(|r| r.vcn < needed).map(|r| r.len.min(needed - r.vcn)).sum();
        if mapped < needed {
            ds.incomplete = true;
        }
        ds
    }

    /// The volume's cluster allocation bitmap (`$Bitmap`, record 6). Bits
    /// that couldn't be read are reported as allocated, so a deleted file
    /// is never called "full" on the strength of unreadable evidence.
    pub fn cluster_bitmap(&mut self) -> Vec<u8> {
        let clusters = self.boot.total_bytes / self.boot.cluster_size;
        let len = clusters.div_ceil(8) as usize;
        let Some(ds) = self.file(6).and_then(|f| f.data) else {
            return vec![0xFF; len];
        };
        let mut bm = vec![0u8; len];
        if let Some(r) = &ds.resident {
            let n = r.len().min(len);
            bm[..n].copy_from_slice(&r[..n]);
            bm[n..].fill(0xFF);
            return bm;
        }
        const PIECE: usize = 64 * 1024;
        for (i, piece) in bm.chunks_mut(PIECE).enumerate() {
            if self.read_runs(&ds.runs, (i * PIECE) as u64, piece) > 0 {
                piece.fill(0xFF);
            }
        }
        bm
    }

    /// Bytes of the stream's clusters (up to its valid length) that are
    /// unreadable, and that are now marked in use in `bitmap`.
    pub fn damage(&self, ds: &DataStream, bitmap: Option<&[u8]>) -> (u64, u64, u64) {
        let cs = self.boot.cluster_size;
        let needed = ds.init_size.div_ceil(cs);
        let (mut total, mut unreadable, mut reused) = (0u64, 0u64, 0u64);
        for r in &ds.runs {
            let Some(lcn) = r.lcn else { continue };
            if r.vcn >= needed {
                continue;
            }
            let len = r.len.min(needed - r.vcn);
            total += len * cs;
            let start = self.part_off + lcn * cs;
            unreadable += self.reader.unreadable_bytes(start, start + len * cs);
            if let Some(bm) = bitmap {
                reused += count_set_bits(bm, lcn, len) * cs;
            }
        }
        (total, unreadable, reused)
    }

    /// Writes the stream's bytes to `out`. Returns (written, unreadable).
    pub fn copy_stream(&mut self, ds: &DataStream, out: &mut impl Write) -> io::Result<(u64, u64)> {
        if let Some(r) = &ds.resident {
            out.write_all(r)?;
            return Ok((r.len() as u64, 0));
        }
        let mut buf = vec![0u8; COPY_CHUNK as usize];
        let mut pos = 0u64;
        let mut bad = 0u64;
        while pos < ds.size {
            let n = (ds.size - pos).min(COPY_CHUNK) as usize;
            let chunk = &mut buf[..n];
            if pos < ds.init_size {
                let valid = ((ds.init_size - pos) as usize).min(n);
                bad += self.read_runs(&ds.runs, pos, &mut chunk[..valid]);
                // Past the valid data length NTFS defines the contents as zero.
                chunk[valid..].fill(0);
            } else {
                chunk.fill(0);
            }
            out.write_all(chunk)?;
            pos += n as u64;
        }
        Ok((ds.size, bad))
    }
}

fn count_set_bits(bm: &[u8], first: u64, len: u64) -> u64 {
    let mut n = 0;
    for c in first..first + len {
        match bm.get((c / 8) as usize) {
            Some(b) if b & (1 << (c % 8)) != 0 => n += 1,
            Some(_) => {}
            None => n += 1, // beyond the volume: not ours to trust
        }
    }
    n
}
