//! Raw access to a Linux block device (`/dev/sdX`) for direct I/O
//! (`O_DIRECT`), bypassing the page cache so a failing drive's real sector
//! health is what the imager sees — the same thing `ddrescue -d` relies on.
//!
//! NOTE: written against documented ioctl numbers (`BLKSSZGET`,
//! `BLKGETSIZE64` from `linux/fs.h`) since the `libc` crate doesn't expose
//! them as constants. Not yet exercised on real Linux hardware — treat as a
//! first draft to validate against an actual `/dev/sdX` before relying on it.

use std::ffi::CString;
use std::io;
use std::os::unix::io::RawFd;

use libc::{c_ulong, close, ioctl, open, pread, O_DIRECT, O_RDONLY};

const BLKSSZGET: c_ulong = 0x1268;
const BLKGETSIZE64: c_ulong = 0x8008_1272;

use crate::block_device::BlockDevice;

pub struct LinuxBlockDevice {
    fd: RawFd,
    size_bytes: u64,
    sector_size: u64,
}

// Safety: the fd is only ever used through &mut self.
unsafe impl Send for LinuxBlockDevice {}

impl LinuxBlockDevice {
    /// Opens a raw device path such as `/dev/sdb`. Requires root (or
    /// membership of the `disk` group) to open for reading.
    pub fn open(path: &str) -> io::Result<Self> {
        let c_path = CString::new(path).map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
        let fd = unsafe { open(c_path.as_ptr(), O_RDONLY | O_DIRECT) };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }

        let mut dev = Self {
            fd,
            size_bytes: 0,
            sector_size: 512,
        };
        dev.size_bytes = dev.query_u64(BLKGETSIZE64)?;
        dev.sector_size = dev.query_u32(BLKSSZGET).unwrap_or(512) as u64;
        Ok(dev)
    }

    fn query_u64(&self, request: c_ulong) -> io::Result<u64> {
        let mut val: u64 = 0;
        let ret = unsafe { ioctl(self.fd, request, &mut val as *mut u64) };
        if ret < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(val)
    }

    fn query_u32(&self, request: c_ulong) -> io::Result<u32> {
        let mut val: u32 = 0;
        let ret = unsafe { ioctl(self.fd, request, &mut val as *mut u32) };
        if ret < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(val)
    }
}

impl BlockDevice for LinuxBlockDevice {
    fn size_bytes(&self) -> u64 {
        self.size_bytes
    }

    fn sector_size(&self) -> u64 {
        self.sector_size
    }

    fn read_at(&mut self, offset: u64, buf: &mut [u8]) -> io::Result<()> {
        let n = unsafe { pread(self.fd, buf.as_mut_ptr() as *mut core::ffi::c_void, buf.len(), offset as i64) };
        if n < 0 {
            return Err(io::Error::last_os_error());
        }
        if n as usize != buf.len() {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "short read from block device",
            ));
        }
        Ok(())
    }
}

impl Drop for LinuxBlockDevice {
    fn drop(&mut self) {
        unsafe {
            close(self.fd);
        }
    }
}
