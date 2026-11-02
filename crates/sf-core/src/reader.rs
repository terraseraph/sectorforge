//! Fault-tolerant reads for the recovery commands (`carve`, `files`,
//! `extract`). Unlike the imager, these never give up on a read error: the
//! unreadable bytes come back as zeros and are *reported*, so callers can say
//! "partial" honestly instead of silently handing back zeros.
//!
//! Ranges the imaging mapfile says were never rescued are treated as
//! unreadable without touching the device: on an image they hold zeros that
//! aren't real data, and on a failing drive re-reading them is wasted wear.

use std::path::Path;

use crate::aligned_buf::AlignedBuf;
use crate::block_device::BlockDevice;
use crate::mapfile::{MapFile, Status};

const MAX_READ: u64 = 4 * 1024 * 1024;
const BUF_ALIGN: usize = 4096;

/// A sorted set of non-overlapping half-open byte ranges.
#[derive(Debug, Clone, Default)]
pub struct RangeSet {
    ranges: Vec<(u64, u64)>,
}

impl RangeSet {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, start: u64, end: u64) {
        if start >= end {
            return;
        }
        let (mut s, mut e) = (start, end);
        let mut out = Vec::with_capacity(self.ranges.len() + 1);
        let mut placed = false;
        for &(rs, re) in &self.ranges {
            if re < s {
                out.push((rs, re));
            } else if e < rs {
                if !placed {
                    out.push((s, e));
                    placed = true;
                }
                out.push((rs, re));
            } else {
                s = s.min(rs);
                e = e.max(re);
            }
        }
        if !placed {
            out.push((s, e));
        }
        self.ranges = out;
    }

    pub fn ranges(&self) -> &[(u64, u64)] {
        &self.ranges
    }

    pub fn is_empty(&self) -> bool {
        self.ranges.is_empty()
    }

    /// Bytes of `[start, end)` covered by the set.
    pub fn bytes_in(&self, start: u64, end: u64) -> u64 {
        self.intersecting(start, end).map(|(s, e)| e - s).sum()
    }

    /// The parts of the set inside `[start, end)`, clipped to it.
    pub fn intersecting(&self, start: u64, end: u64) -> impl Iterator<Item = (u64, u64)> + '_ {
        let first = self.ranges.partition_point(|&(_, re)| re <= start);
        self.ranges[first..]
            .iter()
            .take_while(move |&&(rs, _)| rs < end)
            .map(move |&(rs, re)| (rs.max(start), re.min(end)))
    }
}

/// Byte ranges an imaging map says hold no rescued data: everything not
/// `finished`. Refuses a map made for a different-size source, since its
/// offsets would point at the wrong data.
pub fn unreadable_from_map(path: &Path, source_size: u64) -> Result<RangeSet, String> {
    let map = MapFile::load(path).map_err(|e| format!("cannot read map {}: {e}", path.display()))?;
    if map.size() != source_size {
        return Err(format!(
            "map {} is for a {}-byte source, but this source is {} bytes. Wrong map for this source?",
            path.display(),
            map.size(),
            source_size
        ));
    }
    Ok(unreadable_in(&map))
}

pub fn unreadable_in(map: &MapFile) -> RangeSet {
    let mut set = RangeSet::new();
    for b in map.blocks().iter().filter(|b| b.status != Status::Finished) {
        set.insert(b.pos, b.end());
    }
    set
}

pub struct SafeReader<'a> {
    dev: &'a mut dyn BlockDevice,
    known_bad: RangeSet,
    found_bad: RangeSet,
    /// After a failed read, split down to this many bytes before writing a
    /// range off as bad. Sector size = find every readable sector; larger =
    /// fewer reads of a dying drive at the cost of skipping more.
    granularity: u64,
    buf: AlignedBuf,
}

impl<'a> SafeReader<'a> {
    pub fn new(dev: &'a mut dyn BlockDevice, known_bad: RangeSet) -> Self {
        let sector = dev.sector_size().max(1);
        Self {
            dev,
            known_bad,
            found_bad: RangeSet::new(),
            granularity: sector,
            buf: AlignedBuf::new(MAX_READ as usize, BUF_ALIGN),
        }
    }

    pub fn with_granularity(mut self, bytes: u64) -> Self {
        let sector = self.dev.sector_size().max(1);
        self.granularity = bytes.max(sector).div_ceil(sector) * sector;
        self
    }

    pub fn size(&self) -> u64 {
        self.dev.size_bytes()
    }

    pub fn sector_size(&self) -> u64 {
        self.dev.sector_size()
    }

    /// Every range found unreadable so far, from the map or from failed reads.
    pub fn is_unreadable(&self, start: u64, end: u64) -> bool {
        self.known_bad.bytes_in(start, end) > 0 || self.found_bad.bytes_in(start, end) > 0
    }

    /// How many bytes of `[start, end)` are known to be unreadable, without
    /// reading anything.
    pub fn unreadable_bytes(&self, start: u64, end: u64) -> u64 {
        self.known_bad.bytes_in(start, end) + self.found_bad_not_known(start, end)
    }

    /// Fills `out` with the bytes at `offset`. Returns how many of them were
    /// unreadable (zero-filled): map holes, read errors, or past the end.
    pub fn read(&mut self, offset: u64, out: &mut [u8]) -> u64 {
        out.fill(0);
        let want_end = offset + out.len() as u64;
        let dev_end = self.dev.size_bytes();
        let mut unreadable = want_end.saturating_sub(dev_end.max(offset));
        let end = want_end.min(dev_end);
        if offset >= end {
            return out.len() as u64;
        }

        let sector = self.dev.sector_size().max(1);
        let a_start = offset - offset % sector;
        let a_end = end.div_ceil(sector) * sector;
        let a_end = a_end.min(dev_end.div_ceil(sector) * sector);

        let mut skip = RangeSet::new();
        for (s, e) in self.known_bad.intersecting(a_start, a_end).chain(self.found_bad.intersecting(a_start, a_end)) {
            skip.insert(s, e);
        }
        let mut pos = a_start;
        let mut good = Vec::new();
        for &(s, e) in skip.ranges() {
            if pos < s {
                good.push((pos, s));
            }
            pos = pos.max(e);
        }
        if pos < a_end {
            good.push((pos, a_end));
        }

        for (gs, ge) in good {
            // Known-bad ranges from a map need not be sector-aligned.
            let gs_a = gs - gs % sector;
            let ge_a = ge.div_ceil(sector) * sector;
            let mut p = gs_a;
            while p < ge_a {
                let len = MAX_READ.min(ge_a - p);
                self.read_split(p, len, gs, ge, offset, out);
                p += len;
            }
        }
        unreadable += self.known_bad.bytes_in(offset, end);
        unreadable += self.found_bad_not_known(offset, end);
        unreadable
    }

    fn found_bad_not_known(&self, start: u64, end: u64) -> u64 {
        self.found_bad
            .intersecting(start, end)
            .map(|(s, e)| (e - s) - self.known_bad.bytes_in(s, e))
            .sum()
    }

    /// Reads aligned `[pos, pos+len)`, copying the part inside both
    /// `[keep_s, keep_e)` and the caller's window into `out`. Splits failed
    /// reads in half down to `granularity`.
    fn read_split(&mut self, pos: u64, len: u64, keep_s: u64, keep_e: u64, offset: u64, out: &mut [u8]) {
        let dev_end = self.dev.size_bytes();
        let read_len = len.min(dev_end.saturating_sub(pos).div_ceil(self.dev.sector_size()) * self.dev.sector_size());
        if read_len == 0 {
            return;
        }
        let ok = {
            let buf = &mut self.buf[..read_len as usize];
            self.dev.read_at(pos, buf).is_ok()
        };
        if ok {
            let s = pos.max(keep_s).max(offset);
            let e = (pos + read_len).min(keep_e).min(offset + out.len() as u64).min(dev_end);
            if s < e {
                out[(s - offset) as usize..(e - offset) as usize]
                    .copy_from_slice(&self.buf[(s - pos) as usize..(e - pos) as usize]);
            }
            return;
        }
        let sector = self.dev.sector_size();
        if len <= self.granularity || len <= sector {
            self.found_bad.insert(pos, (pos + len).min(dev_end));
            return;
        }
        let half = (len / 2).div_ceil(sector) * sector;
        self.read_split(pos, half, keep_s, keep_e, offset, out);
        self.read_split(pos + half, len - half, keep_s, keep_e, offset, out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block_device::FakeBlockDevice;

    fn dev(size: usize) -> FakeBlockDevice {
        FakeBlockDevice::new((0..size).map(|i| (i % 251) as u8).collect(), 512)
    }

    #[test]
    fn range_set_merges_and_measures() {
        let mut r = RangeSet::new();
        r.insert(10, 20);
        r.insert(30, 40);
        r.insert(15, 32);
        assert_eq!(r.ranges(), &[(10, 40)]);
        r.insert(0, 5);
        assert_eq!(r.ranges(), &[(0, 5), (10, 40)]);
        assert_eq!(r.bytes_in(3, 12), 4);
    }

    #[test]
    fn unaligned_read_matches_device() {
        let mut d = dev(8192);
        let mut r = SafeReader::new(&mut d, RangeSet::new());
        let mut buf = vec![0u8; 1000];
        assert_eq!(r.read(777, &mut buf), 0);
        assert!(buf.iter().enumerate().all(|(i, &b)| b == ((777 + i) % 251) as u8));
    }

    #[test]
    fn bad_sectors_come_back_zeroed_and_counted() {
        let mut d = dev(8192);
        d.mark_bad(1024, 1536);
        let mut r = SafeReader::new(&mut d, RangeSet::new());
        let mut buf = vec![0u8; 4096];
        assert_eq!(r.read(0, &mut buf), 512);
        assert!(buf[1024..1536].iter().all(|&b| b == 0));
        assert_eq!(buf[1536], (1536 % 251) as u8);
        assert!(r.is_unreadable(1100, 1101));
    }

    #[test]
    fn map_holes_are_unreadable_without_reading() {
        let mut d = dev(8192);
        let mut bad = RangeSet::new();
        bad.insert(100, 200);
        let mut r = SafeReader::new(&mut d, bad);
        let mut buf = vec![0u8; 512];
        assert_eq!(r.read(0, &mut buf), 100);
        assert!(buf[100..200].iter().all(|&b| b == 0));
        assert_eq!(buf[99], 99);
        assert_eq!(buf[200], (200 % 251) as u8);
    }

    #[test]
    fn past_the_end_is_unreadable() {
        let mut d = dev(1024);
        let mut r = SafeReader::new(&mut d, RangeSet::new());
        let mut buf = vec![0u8; 1024];
        assert_eq!(r.read(512, &mut buf), 512);
    }
}
