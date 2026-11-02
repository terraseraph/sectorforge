//! Raw access to a Windows physical drive (`\\.\PhysicalDriveN`) for direct,
//! unbuffered reads — bypasses the OS cache so a failing drive's real sector
//! health is what the imager actually sees, matching what `ddrescue -d`
//! does on Linux. Requires administrator privileges to open.
//!
//! The SCSI and ATA modes send READ commands through
//! IOCTL_SCSI/ATA_PASS_THROUGH_DIRECT instead, each with its own timeout,
//! so a bad sector costs seconds rather than the disk driver's minutes of
//! retries. Windows only allows pass-through on a handle opened for writing,
//! but the only commands ever built here are fixed read opcodes.

use std::ffi::OsStr;
use std::io;
use std::os::windows::ffi::OsStrExt;
use std::ptr::null_mut;

use std::time::Duration;

use windows_sys::Win32::Foundation::{CloseHandle, GENERIC_READ, GENERIC_WRITE, HANDLE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::Storage::IscsiDisc::{
    ATA_FLAGS_48BIT_COMMAND, ATA_FLAGS_DATA_IN, ATA_FLAGS_DRDY_REQUIRED, ATA_FLAGS_USE_DMA, ATA_PASS_THROUGH_DIRECT,
    IOCTL_ATA_PASS_THROUGH_DIRECT, IOCTL_SCSI_PASS_THROUGH_DIRECT, SCSI_IOCTL_DATA_IN, SCSI_PASS_THROUGH_DIRECT,
};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, ReadFile, SetFilePointerEx, FILE_BEGIN, FILE_FLAG_NO_BUFFERING, FILE_GENERIC_READ,
    FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
};
use windows_sys::Win32::System::Ioctl::{
    DISK_GEOMETRY_EX, FSCTL_SET_SPARSE, GET_LENGTH_INFORMATION, IOCTL_DISK_GET_DRIVE_GEOMETRY_EX,
    IOCTL_DISK_GET_LENGTH_INFO,
};
use windows_sys::Win32::System::IO::DeviceIoControl;

use super::Access;
use crate::block_device::BlockDevice;

/// Largest single pass-through command. Adapters' transfer limits vary and
/// some are as low as 64 KiB; one read_at is split into commands this size.
const PASS_THROUGH_MAX: usize = 64 * 1024;

pub struct WindowsPhysicalDrive {
    handle: HANDLE,
    size_bytes: u64,
    sector_size: u64,
    access: Access,
    timeout_secs: u32,
}

// Safety: the handle is only ever used through &mut self, so there is no
// concurrent access; moving it to another thread (Send) is sound.
unsafe impl Send for WindowsPhysicalDrive {}

impl WindowsPhysicalDrive {
    /// Opens a raw device path such as `\\.\PhysicalDrive1` for direct,
    /// unbuffered reads. Requires administrator privileges.
    pub fn open(path: &str) -> io::Result<Self> {
        Self::open_with(path, Access::Os, Duration::from_secs(10))
    }

    /// Like [`open`](Self::open), reading through `access`. `timeout` is
    /// per command and only applies to the pass-through modes.
    pub fn open_with(path: &str, access: Access, timeout: Duration) -> io::Result<Self> {
        let wide: Vec<u16> = OsStr::new(path).encode_wide().chain(std::iter::once(0)).collect();
        let handle = unsafe {
            CreateFileW(
                wide.as_ptr(),
                if access == Access::Os { FILE_GENERIC_READ } else { GENERIC_READ | GENERIC_WRITE },
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
            access,
            timeout_secs: timeout.as_secs().clamp(1, u32::MAX as u64) as u32,
        };
        drive.size_bytes = drive.query_size()?;
        drive.sector_size = drive.query_sector_size().unwrap_or(512);
        if access != Access::Os {
            // Fail now, with an explanation, rather than on every read of the run.
            let mut probe = crate::aligned_buf::AlignedBuf::new(drive.sector_size as usize, drive.sector_size as usize);
            drive.read_at(0, &mut probe).map_err(|e| {
                let (name, hint) = match access {
                    Access::Ata => ("ATA", "NVMe drives and most USB enclosures don't support it"),
                    _ => ("SCSI", "some NVMe drivers and USB bridges don't support it"),
                };
                io::Error::new(e.kind(), format!("{name} pass-through doesn't work on this drive ({e}); {hint}. Use OS access instead"))
            })?;
        }
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

impl WindowsPhysicalDrive {
    fn ioctl<T>(&self, code: u32, req: &mut T) -> io::Result<()> {
        let size = std::mem::size_of::<T>() as u32;
        let mut returned = 0u32;
        let p = req as *mut T as *mut core::ffi::c_void;
        if unsafe { DeviceIoControl(self.handle, code, p, size, p, size, &mut returned, null_mut()) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    /// SCSI READ(16) of `buf.len()` bytes at `lba`.
    fn scsi_read(&self, lba: u64, buf: &mut [u8]) -> io::Result<()> {
        #[repr(C)]
        struct Request {
            sptd: SCSI_PASS_THROUGH_DIRECT,
            sense: [u8; 32],
        }
        let mut r: Request = unsafe { std::mem::zeroed() };
        r.sptd.Length = std::mem::size_of::<SCSI_PASS_THROUGH_DIRECT>() as u16;
        r.sptd.CdbLength = 16;
        r.sptd.SenseInfoLength = r.sense.len() as u8;
        r.sptd.SenseInfoOffset = std::mem::offset_of!(Request, sense) as u32;
        r.sptd.DataIn = SCSI_IOCTL_DATA_IN as u8;
        r.sptd.DataTransferLength = buf.len() as u32;
        r.sptd.TimeOutValue = self.timeout_secs;
        r.sptd.DataBuffer = buf.as_mut_ptr().cast();
        r.sptd.Cdb[0] = 0x88; // READ(16)
        r.sptd.Cdb[2..10].copy_from_slice(&lba.to_be_bytes());
        r.sptd.Cdb[10..14].copy_from_slice(&((buf.len() as u64 / self.sector_size) as u32).to_be_bytes());
        self.ioctl(IOCTL_SCSI_PASS_THROUGH_DIRECT, &mut r)?;
        if r.sptd.ScsiStatus != 0 {
            let (key, asc, ascq) = sense_codes(&r.sense);
            return Err(io::Error::other(format!(
                "drive reported a read error (SCSI status 0x{:02X}, sense key {key:X}, ASC 0x{asc:02X}, ASCQ 0x{ascq:02X})",
                r.sptd.ScsiStatus
            )));
        }
        if r.sptd.DataTransferLength as usize != buf.len() {
            return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "short read from physical drive"));
        }
        Ok(())
    }

    /// ATA READ DMA EXT of `buf.len()` bytes at `lba` (48-bit addressing).
    fn ata_read(&self, lba: u64, buf: &mut [u8]) -> io::Result<()> {
        let count = buf.len() as u64 / self.sector_size; // at most 128, well inside the 16-bit count
        let mut a: ATA_PASS_THROUGH_DIRECT = unsafe { std::mem::zeroed() };
        a.Length = std::mem::size_of::<ATA_PASS_THROUGH_DIRECT>() as u16;
        a.AtaFlags = (ATA_FLAGS_DRDY_REQUIRED | ATA_FLAGS_DATA_IN | ATA_FLAGS_48BIT_COMMAND | ATA_FLAGS_USE_DMA) as u16;
        a.DataTransferLength = buf.len() as u32;
        a.TimeOutValue = self.timeout_secs;
        a.DataBuffer = buf.as_mut_ptr().cast();
        let b = |shift: u32| (lba >> shift) as u8;
        // [0] features, [1] count, [2..5] LBA, [5] device, [6] command; "previous" holds the high bytes.
        a.PreviousTaskFile = [0, (count >> 8) as u8, b(24), b(32), b(40), 0, 0, 0];
        a.CurrentTaskFile = [0, count as u8, b(0), b(8), b(16), 0x40, 0x25, 0];
        self.ioctl(IOCTL_ATA_PASS_THROUGH_DIRECT, &mut a)?;
        // Afterwards [6] is the status register (ERR 0x01, DF 0x20) and [0] the error register.
        let status = a.CurrentTaskFile[6];
        if status & 0x21 != 0 {
            return Err(io::Error::other(format!(
                "drive reported a read error (ATA status 0x{status:02X}, error 0x{:02X})",
                a.CurrentTaskFile[0]
            )));
        }
        Ok(())
    }
}

pub(super) fn set_sparse(file: &std::fs::File) -> io::Result<()> {
    use std::os::windows::io::AsRawHandle;
    let mut returned = 0u32;
    let ok = unsafe {
        DeviceIoControl(file.as_raw_handle(), FSCTL_SET_SPARSE, null_mut(), 0, null_mut(), 0, &mut returned, null_mut())
    };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// Sense key, ASC and ASCQ from fixed (0x70/0x71) or descriptor (0x72/0x73) sense data.
fn sense_codes(s: &[u8; 32]) -> (u8, u8, u8) {
    match s[0] & 0x7F {
        0x72 | 0x73 => (s[1] & 0x0F, s[2], s[3]),
        _ => (s[2] & 0x0F, s[12], s[13]),
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
        if self.access != Access::Os {
            let mut lba = offset / self.sector_size;
            for piece in buf.chunks_mut(PASS_THROUGH_MAX) {
                match self.access {
                    Access::Ata => self.ata_read(lba, piece)?,
                    _ => self.scsi_read(lba, piece)?,
                }
                lba += piece.len() as u64 / self.sector_size;
            }
            return Ok(());
        }
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
