//! SectorForge's recovery engine: cross-platform raw disk access, a
//! resumable ddrescue-style mapfile, and the imaging algorithm that uses
//! them. Platform GUIs and CLIs are thin shells over this crate.

pub mod aligned_buf;
pub mod block_device;
pub mod carve;
pub mod device;
pub mod drives;
pub mod ext;
pub mod fat;
pub mod imager;
pub mod mapfile;
pub mod ntfs;
pub mod partitions;
pub mod reader;
pub mod sanitize;
pub mod timefmt;

pub use block_device::BlockDevice;
pub use imager::{Imager, ImagerConfig, Progress};
pub use mapfile::{Block, MapFile, Status};
pub use partitions::{Confidence, FsKind, PartitionInfo, PartitionSource, ScanResult, TableKind};

use std::io;

// Lets callers hold a `Box<dyn BlockDevice>` (e.g. to pick a backend at
// runtime) and still use it directly with `Imager<D: BlockDevice, _>`.
impl BlockDevice for Box<dyn BlockDevice> {
    fn size_bytes(&self) -> u64 {
        (**self).size_bytes()
    }

    fn sector_size(&self) -> u64 {
        (**self).sector_size()
    }

    fn read_at(&mut self, offset: u64, buf: &mut [u8]) -> io::Result<()> {
        (**self).read_at(offset, buf)
    }
}
