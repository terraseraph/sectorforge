//! A `BlockDevice` backed by a plain file — either an existing disk image
//! (to re-run partition/file recovery without touching real hardware again)
//! or, in tests and development, a stand-in for a physical drive. This is
//! "Image file" mode: the counterpart to the raw `\\.\PhysicalDriveN` /
//! `/dev/sdX` backends, which are "direct hardware access" mode.

use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};

use crate::block_device::BlockDevice;

pub struct FileBlockDevice {
    file: File,
    size_bytes: u64,
    sector_size: u64,
}

impl FileBlockDevice {
    /// Opens `path` as a block device. `sector_size` only affects how the
    /// imager chunks its reads — a plain file has no real alignment
    /// requirement, so 512 is a safe default when the size is unknown.
    pub fn open(path: &str, sector_size: u64) -> io::Result<Self> {
        let file = File::open(path)?;
        let size_bytes = file.metadata()?.len();
        Ok(Self {
            file,
            size_bytes,
            sector_size,
        })
    }
}

impl BlockDevice for FileBlockDevice {
    fn size_bytes(&self) -> u64 {
        self.size_bytes
    }

    fn sector_size(&self) -> u64 {
        self.sector_size
    }

    fn read_at(&mut self, offset: u64, buf: &mut [u8]) -> io::Result<()> {
        self.file.seek(SeekFrom::Start(offset))?;
        self.file.read_exact(buf)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_back_what_was_written() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("fake_disk.img");
        std::fs::write(&path, (0..4096u32).map(|i| (i % 256) as u8).collect::<Vec<u8>>()).unwrap();

        let mut dev = FileBlockDevice::open(path.to_str().unwrap(), 512).unwrap();
        assert_eq!(dev.size_bytes(), 4096);

        let mut buf = vec![0u8; 512];
        dev.read_at(1024, &mut buf).unwrap();
        let expected: Vec<u8> = (1024..1536u32).map(|i| (i % 256) as u8).collect();
        assert_eq!(buf, expected);
    }
}
