//! Raw access to a Windows physical drive (`\\.\PhysicalDriveN`) for direct,
//! unbuffered reads — bypasses the OS cache so a failing drive's real sector
//! health is what the imager actually sees, matching what `ddrescue -d`
//! does on Linux. Requires administrator privileges to open.

use std::ffi::OsStr;
use std::io;
use std::os::windows::ffi::OsStrExt;
use std::ptr::null_mut;

use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, ReadFile, SetFilePointerEx, FILE_BEGIN, FILE_FLAG_NO_BUFFERING, FILE_GENERIC_READ,
    FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
};
use windows_sys::Win32::System::Ioctl::{
    DISK_GEOMETRY_EX, GET_LENGTH_INFORMATION, IOCTL_DISK_GET_DRIVE_GEOMETRY_EX,
    IOCTL_DISK_GET_LENGTH_INFO,
};
use windows_sys::Win32::System::IO::DeviceIoControl;

use crate::block_device::BlockDevice;

pub struct WindowsPhysicalDrive {
    handle: HANDLE,
    size_bytes: u64,
    sector_size: u64,
}

// Safety: the handle is only ever used through &mut self, so there is no
// concurrent access; moving it to another thread (Send) is sound.
unsafe impl Send for WindowsPhysicalDrive {}

impl WindowsPhysicalDrive {
    /// Opens a raw device path such as `\\.\PhysicalDrive1` for direct,
    /// unbuffered reads. Requires administrator privileges.
    pub fn open(path: &str) -> io::Result<Self> {
        let wide: Vec<u16> = OsStr::new(path).encode_wide().chain(std::iter::once(0)).collect();
        let handle = unsafe {
            CreateFileW(
                wide.as_ptr(),
                FILE_GENERIC_READ,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                null_mut(),
                OPEN_EXISTING,
                FILE_FLAG_NO_BUFFERING,
                null_mut(),
            )
        };
        if handle == INVALID_HANDLE_VALUE {
            return Err(io::Error::last_os_error());
        }

        let mut drive = Self {
            handle,
            size_bytes: 0,
            sector_size: 512,
        };
        drive.size_bytes = drive.query_size()?;
        drive.sector_size = drive.query_sector_size().unwrap_or(512);
        Ok(drive)
    }

    fn query_size(&self) -> io::Result<u64> {
        let mut info: GET_LENGTH_INFORMATION = unsafe { std::mem::zeroed() };
        let mut bytes_returned: u32 = 0;
        let ok = unsafe {
            DeviceIoControl(
                self.handle,
                IOCTL_DISK_GET_LENGTH_INFO,
                null_mut(),
                0,
                &mut info as *mut _ as *mut core::ffi::c_void,
                std::mem::size_of::<GET_LENGTH_INFORMATION>() as u32,
                &mut bytes_returned,
                null_mut(),
            )
        };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(info.Length as u64)
    }

    fn query_sector_size(&self) -> io::Result<u64> {
        let mut geo: DISK_GEOMETRY_EX = unsafe { std::mem::zeroed() };
        let mut bytes_returned: u32 = 0;
        let ok = unsafe {
            DeviceIoControl(
                self.handle,
                IOCTL_DISK_GET_DRIVE_GEOMETRY_EX,
                null_mut(),
                0,
                &mut geo as *mut _ as *mut core::ffi::c_void,
                std::mem::size_of::<DISK_GEOMETRY_EX>() as u32,
                &mut bytes_returned,
                null_mut(),
            )
        };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(geo.Geometry.BytesPerSector as u64)
    }
}

impl BlockDevice for WindowsPhysicalDrive {
    fn size_bytes(&self) -> u64 {
        self.size_bytes
    }

    fn sector_size(&self) -> u64 {
        self.sector_size
    }

    fn read_at(&mut self, offset: u64, buf: &mut [u8]) -> io::Result<()> {
        unsafe {
            let mut new_pos: i64 = 0;
            let ok = SetFilePointerEx(self.handle, offset as i64, &mut new_pos, FILE_BEGIN);
            if ok == 0 {
                return Err(io::Error::last_os_error());
            }
            let mut bytes_read: u32 = 0;
            let ok = ReadFile(
                self.handle,
                buf.as_mut_ptr(),
                buf.len() as u32,
                &mut bytes_read,
                null_mut(),
            );
            if ok == 0 {
                return Err(io::Error::last_os_error());
            }
            if bytes_read as usize != buf.len() {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "short read from physical drive",
                ));
            }
        }
        Ok(())
    }
}

impl Drop for WindowsPhysicalDrive {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.handle);
        }
    }
}
