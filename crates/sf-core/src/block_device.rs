//! The `BlockDevice` abstraction that the imaging engine reads from.
//!
//! Every platform-specific implementation (raw Windows physical drive, raw
//! Linux block device) and the in-memory fake used by tests implement this
//! same trait, so `Imager` never needs to know which one it's holding.

use std::io;

/// A readable block device: a source disk, or a stand-in for one in tests.
///
/// All reads must be issued at offsets and lengths that are multiples of
/// [`BlockDevice::sector_size`] — that's a hard requirement of unbuffered
/// (direct) I/O on real hardware, so the trait enforces it everywhere rather
/// than only on the platforms that need it.
pub trait BlockDevice: Send {
    /// Total size of the device in bytes.
    fn size_bytes(&self) -> u64;

    /// The device's logical sector size in bytes (e.g. 512 or 4096). All
    /// `read_at` offsets and buffer lengths must be a multiple of this.
    fn sector_size(&self) -> u64;

    /// Reads exactly `buf.len()` bytes starting at `offset` into `buf`.
    ///
    /// On a real failing drive a read either succeeds in full or fails in
    /// full for the requested range — there is no well-defined "partial
    /// sector" result — so implementations should treat any OS-reported
    /// short read over a direct-I/O handle as an error for the remainder.
    fn read_at(&mut self, offset: u64, buf: &mut [u8]) -> io::Result<()>;
}

/// An in-memory `BlockDevice` for tests: backed by a byte buffer, with a set
/// of byte ranges that always fail as if they were bad sectors.
#[derive(Clone)]
pub struct FakeBlockDevice {
    data: Vec<u8>,
    sector_size: u64,
    bad_ranges: Vec<(u64, u64)>, // (start, end) exclusive, byte offsets
    /// Number of read_at calls made so far (useful for asserting retry counts in tests).
    pub read_count: u64,
}

impl FakeBlockDevice {
    pub fn new(data: Vec<u8>, sector_size: u64) -> Self {
        assert!(sector_size.is_power_of_two());
        Self {
            data,
            sector_size,
            bad_ranges: Vec::new(),
            read_count: 0,
        }
    }

    /// Marks `[start, end)` (byte offsets) as always failing to read.
    pub fn mark_bad(&mut self, start: u64, end: u64) {
        self.bad_ranges.push((start, end));
    }

    fn overlaps_bad(&self, start: u64, end: u64) -> bool {
        self.bad_ranges
            .iter()
            .any(|&(bs, be)| start < be && bs < end)
    }
}

impl BlockDevice for FakeBlockDevice {
    fn size_bytes(&self) -> u64 {
        self.data.len() as u64
    }

    fn sector_size(&self) -> u64 {
        self.sector_size
    }

    fn read_at(&mut self, offset: u64, buf: &mut [u8]) -> io::Result<()> {
        self.read_count += 1;
        assert_eq!(offset % self.sector_size, 0, "unaligned offset in test device");
        assert_eq!(
            buf.len() as u64 % self.sector_size,
            0,
            "unaligned length in test device"
        );
        let end = offset + buf.len() as u64;
        if end > self.size_bytes() {
            return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "read past end of device"));
        }
        if self.overlaps_bad(offset, end) {
            return Err(io::Error::new(io::ErrorKind::Other, "simulated bad sector"));
        }
        let start = offset as usize;
        let len = buf.len();
        buf.copy_from_slice(&self.data[start..start + len]);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_good_region() {
        let data: Vec<u8> = (0..1024u32).map(|i| (i % 256) as u8).collect();
        let mut dev = FakeBlockDevice::new(data, 512);
        let mut buf = vec![0u8; 512];
        dev.read_at(0, &mut buf).unwrap();
        assert_eq!(buf[0], 0);
        assert_eq!(buf[1], 1);
    }

    #[test]
    fn fails_on_bad_range() {
        let data = vec![0u8; 2048];
        let mut dev = FakeBlockDevice::new(data, 512);
        dev.mark_bad(512, 1024);
        let mut buf = vec![0u8; 512];
        assert!(dev.read_at(0, &mut buf).is_ok());
        assert!(dev.read_at(512, &mut buf).is_err());
        assert!(dev.read_at(1024, &mut buf).is_ok());
    }

    #[test]
    fn fails_past_end() {
        let data = vec![0u8; 512];
        let mut dev = FakeBlockDevice::new(data, 512);
        let mut buf = vec![0u8; 512];
        assert!(dev.read_at(512, &mut buf).is_err());
    }
}
