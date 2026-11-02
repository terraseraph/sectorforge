//! The cloning engine: reads a [`BlockDevice`] forward, skipping past damage
//! quickly on a first pass (maximize rescued data fast, like ddrescue's
//! phase 1), then narrows down exactly which sectors are bad on a retry pass
//! (like ddrescue's trim/scrape phases). Every outcome is recorded in a
//! [`MapFile`] so a run can be stopped and resumed at any point.

use std::io::{self, Seek, SeekFrom, Write};

use crate::aligned_buf::AlignedBuf;
use crate::block_device::BlockDevice;
use crate::mapfile::{MapFile, Status};

#[derive(Debug, Clone, Copy)]
pub struct ImagerConfig {
    /// Preferred read size on the fast first pass, before any alignment.
    pub initial_chunk: u64,
    /// How far past a failed chunk to jump before continuing the first
    /// pass, so one bad region doesn't stall progress on the rest of the
    /// disk. Set to `initial_chunk` for "don't skip, trim immediately".
    pub skip_size: u64,
    /// How many times to retry a single sector before giving up on it as
    /// genuinely bad.
    pub max_retries: u32,
}

impl Default for ImagerConfig {
    fn default() -> Self {
        Self {
            initial_chunk: 1024 * 1024, // 1 MiB
            skip_size: 1024 * 1024,
            max_retries: 3,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Progress {
    pub current_pos: u64,
    pub bytes_total: u64,
    pub bytes_rescued: u64,
    pub bytes_bad: u64,
    pub bytes_pending: u64,
}

pub struct Imager<D: BlockDevice, W: Write + Seek> {
    device: D,
    dest: W,
    map: MapFile,
    config: ImagerConfig,
}

impl<D: BlockDevice, W: Write + Seek> Imager<D, W> {
    pub fn new(device: D, dest: W, map: MapFile, config: ImagerConfig) -> Self {
        Self {
            device,
            dest,
            map,
            config,
        }
    }

    pub fn map(&self) -> &MapFile {
        &self.map
    }

    /// Consumes the imager, handing back the map (e.g. to save it once
    /// `device`/`dest` are no longer needed).
    pub fn into_map(self) -> MapFile {
        self.map
    }

    pub fn save_map(&self, path: impl AsRef<std::path::Path>) -> io::Result<()> {
        self.map.save(path)
    }

    fn progress(&self) -> Progress {
        let total = self.map.size();
        Progress {
            current_pos: self.map.current_pos,
            bytes_total: total,
            bytes_rescued: self.map.bytes_with_status(Status::Finished),
            bytes_bad: self.map.bytes_with_status(Status::BadSector),
            bytes_pending: total
                - self.map.bytes_with_status(Status::Finished)
                - self.map.bytes_with_status(Status::BadSector),
        }
    }

    fn write_chunk(&mut self, pos: u64, data: &[u8]) -> io::Result<()> {
        self.dest.seek(SeekFrom::Start(pos))?;
        self.dest.write_all(data)
    }

    /// Phase 1: sweep forward over every not-yet-tried region in large
    /// chunks, copying what reads cleanly and marking the rest
    /// [`Status::NonTrimmed`] for the retry pass, without stopping to dig
    /// into *why* a chunk failed. This is what gets the bulk of a failing
    /// drive's data off quickly, before the drive gets any worse.
    pub fn run_copy_pass(&mut self, mut on_progress: impl FnMut(Progress)) -> io::Result<()> {
        let sector = self.device.sector_size();
        let mut pos = align_down(self.map.current_pos, sector);

        loop {
            let Some((found_pos, avail)) = self.map.find_next(pos, &[Status::NonTried]) else {
                break;
            };
            pos = found_pos;

            let want = self.config.initial_chunk.min(avail);
            let chunk_len = align_chunk_len(want, sector);

            let mut buf = AlignedBuf::new(chunk_len as usize, sector as usize);
            match self.device.read_at(pos, &mut buf) {
                Ok(()) => {
                    self.write_chunk(pos, &buf)?;
                    self.map.mark(pos, chunk_len, Status::Finished);
                    pos += chunk_len;
                }
                Err(_) => {
                    // Don't dig in yet — record the whole attempted range as
                    // damaged-but-unexamined and jump past it, so one bad
                    // spot can't stall the rescue of everything after it.
                    let skip = align_chunk_len(self.config.skip_size.max(chunk_len), sector).min(avail);
                    self.map.mark(pos, skip, Status::NonTrimmed);
                    pos += skip;
                }
            }

            self.map.current_pos = pos;
            self.map.current_status = Status::NonTried;
            on_progress(self.progress());
        }
        Ok(())
    }

    /// Phase 2: for every region the first pass couldn't read, binary-split
    /// it down toward individual sectors so only the sectors that are truly
    /// unreadable end up marked [`Status::BadSector`] — everything else in
    /// that chunk still gets rescued.
    pub fn run_retry_pass(&mut self, mut on_progress: impl FnMut(Progress)) -> io::Result<()> {
        let sector = self.device.sector_size();

        loop {
            let Some((pos, len)) = self
                .map
                .find_next(0, &[Status::NonTrimmed, Status::NonScraped])
            else {
                break;
            };
            self.split_and_retry(pos, len, sector, 0)?;
            on_progress(self.progress());
        }
        Ok(())
    }

    fn split_and_retry(&mut self, pos: u64, len: u64, sector: u64, attempt: u32) -> io::Result<()> {
        let mut buf = AlignedBuf::new(len as usize, sector as usize);
        match self.device.read_at(pos, &mut buf) {
            Ok(()) => {
                self.write_chunk(pos, &buf)?;
                self.map.mark(pos, len, Status::Finished);
                return Ok(());
            }
            Err(_) => {
                if len <= sector {
                    if attempt + 1 < self.config.max_retries {
                        return self.split_and_retry(pos, len, sector, attempt + 1);
                    }
                    self.map.mark(pos, len, Status::BadSector);
                    return Ok(());
                }
            }
        }

        // Split into two halves, aligned to the sector size, and retry each
        // independently — isolates the damage instead of giving up on the
        // whole chunk.
        let half = align_chunk_len(len / 2, sector).max(sector);
        let half = half.min(len - sector); // leave at least one sector for the second half
        self.map.mark(pos, len, Status::NonScraped);
        self.split_and_retry(pos, half, sector, 0)?;
        self.split_and_retry(pos + half, len - half, sector, 0)
    }
}

fn align_down(x: u64, align: u64) -> u64 {
    x - (x % align)
}

/// Rounds a desired chunk length down to a multiple of `align`, but never
/// below `align` itself (so a single sector can always be attempted).
fn align_chunk_len(len: u64, align: u64) -> u64 {
    let aligned = len - (len % align);
    aligned.max(align)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block_device::FakeBlockDevice;
    use std::io::Cursor;

    fn make_device(size: u64, sector: u64) -> FakeBlockDevice {
        let data: Vec<u8> = (0..size).map(|i| (i % 251) as u8).collect();
        FakeBlockDevice::new(data, sector)
    }

    #[test]
    fn copy_pass_rescues_a_clean_device_fully() {
        let device = make_device(8192, 512);
        let expected = (0..8192u64).map(|i| (i % 251) as u8).collect::<Vec<u8>>();
        let map = MapFile::new_non_tried(8192);
        let dest = Cursor::new(vec![0u8; 8192]);
        let config = ImagerConfig {
            initial_chunk: 2048,
            skip_size: 2048,
            max_retries: 3,
        };
        let mut imager = Imager::new(device, dest, map, config);
        imager.run_copy_pass(|_| {}).unwrap();

        assert_eq!(imager.map().bytes_with_status(Status::Finished), 8192);
        assert_eq!(imager.dest.get_ref(), &expected);
    }

    #[test]
    fn copy_then_retry_isolates_bad_sectors_and_rescues_the_rest() {
        let mut device = make_device(8192, 512);
        // A two-sector bad patch in the middle of an otherwise good chunk.
        device.mark_bad(3072, 4096);

        let map = MapFile::new_non_tried(8192);
        let dest = Cursor::new(vec![0u8; 8192]);
        let config = ImagerConfig {
            initial_chunk: 4096,
            skip_size: 4096,
            max_retries: 2,
        };
        let mut imager = Imager::new(device, dest, map, config);
        imager.run_copy_pass(|_| {}).unwrap();

        // The 4096-byte chunk containing the bad patch couldn't be read as a
        // whole, so it's parked for the retry pass; nothing is lost yet.
        assert_eq!(imager.map().bytes_with_status(Status::BadSector), 0);
        assert!(imager.map().bytes_with_status(Status::NonTrimmed) > 0);

        imager.run_retry_pass(|_| {}).unwrap();

        let map = imager.map();
        assert_eq!(map.bytes_with_status(Status::BadSector), 1024); // exactly the two bad sectors
        assert_eq!(map.bytes_with_status(Status::Finished), 8192 - 1024);
        assert_eq!(map.bytes_with_status(Status::NonTried), 0);
        assert_eq!(map.bytes_with_status(Status::NonTrimmed), 0);
        assert_eq!(map.bytes_with_status(Status::NonScraped), 0);
    }

    #[test]
    fn resuming_from_a_saved_map_does_not_redo_finished_work() {
        let device = make_device(4096, 512);
        let mut map = MapFile::new_non_tried(4096);
        map.mark(0, 2048, Status::Finished); // pretend a previous run already did this half
        map.current_pos = 2048;

        let dest = Cursor::new(vec![0u8; 4096]);
        let config = ImagerConfig::default();
        let mut imager = Imager::new(device, dest, map, config);
        imager.run_copy_pass(|_| {}).unwrap();

        // The destination buffer should be untouched in the already-finished
        // region (still zero) and filled in from 2048 onward.
        let out = imager.dest.get_ref();
        assert!(out[0..2048].iter().all(|&b| b == 0));
        assert_eq!(&out[2048..4096], &(2048u64..4096).map(|i| (i % 251) as u8).collect::<Vec<u8>>()[..]);
        assert_eq!(imager.map().bytes_with_status(Status::Finished), 4096);
    }
}
