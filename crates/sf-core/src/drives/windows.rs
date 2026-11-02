//! Windows: physical drives via `\\.\PhysicalDriveN` opened with *zero*
//! desired access (enough for the property/geometry IOCTLs, and allowed
//! without Administrator), volumes → disks via disk extents.

use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use std::ptr::null_mut;

use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, HANDLE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::Security::{GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, GetDriveTypeW, GetVolumeNameForVolumeMountPointW, GetVolumePathNameW, FILE_SHARE_READ, FILE_SHARE_WRITE,
    OPEN_EXISTING,
};
use windows_sys::Win32::System::Ioctl::{
    DISK_GEOMETRY_EX, IOCTL_DISK_GET_DRIVE_GEOMETRY_EX, IOCTL_STORAGE_QUERY_PROPERTY,
};
use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
use windows_sys::Win32::System::IO::DeviceIoControl;

use super::{Backing, DriveInfo};

const MAX_DRIVES: u32 = 64;
// CTL_CODE(IOCTL_VOLUME_BASE, 0, METHOD_BUFFERED, FILE_ANY_ACCESS); not in windows-sys.
const IOCTL_VOLUME_GET_VOLUME_DISK_EXTENTS: u32 = 0x0056_0000;
const DRIVE_REMOTE: u32 = 4;
const DRIVE_RAMDISK: u32 = 6;

struct Handle(HANDLE);

impl Drop for Handle {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.0) };
    }
}

fn wide(s: &str) -> Vec<u16> {
    OsStr::new(s).encode_wide().chain(std::iter::once(0)).collect()
}

fn open_no_access(path: &str) -> Option<Handle> {
    let w = wide(path);
    let h = unsafe {
        CreateFileW(w.as_ptr(), 0, FILE_SHARE_READ | FILE_SHARE_WRITE, null_mut(), OPEN_EXISTING, 0, null_mut())
    };
    (h != INVALID_HANDLE_VALUE).then_some(Handle(h))
}

fn ioctl(h: &Handle, code: u32, input: &[u8], out_len: usize) -> Option<Vec<u8>> {
    let mut out = vec![0u8; out_len];
    let mut returned = 0u32;
    let ok = unsafe {
        DeviceIoControl(
            h.0,
            code,
            input.as_ptr() as *const _,
            input.len() as u32,
            out.as_mut_ptr() as *mut _,
            out.len() as u32,
            &mut returned,
            null_mut(),
        )
    };
    if ok == 0 {
        return None;
    }
    out.truncate(returned as usize);
    Some(out)
}

pub fn list() -> std::io::Result<Vec<DriveInfo>> {
    let system_disks = std::env::var("SystemDrive")
        .ok()
        .and_then(|d| backing_of_path(Path::new(&format!("{d}\\"))).ok())
        .map(|b| match b {
            Backing::Disks(d) => d,
            Backing::NotLocal => Vec::new(),
        })
        .unwrap_or_default();

    let mut out = Vec::new();
    for n in 0..MAX_DRIVES {
        let path = format!(r"\\.\PhysicalDrive{n}");
        let Some(h) = open_no_access(&path) else { continue };
        // STORAGE_PROPERTY_QUERY { StorageDeviceProperty, PropertyStandardQuery }
        let query = [0u8; 12];
        let desc = ioctl(&h, IOCTL_STORAGE_QUERY_PROPERTY, &query, 4096).and_then(|d| parse_device_descriptor(&d));
        let geo = ioctl(&h, IOCTL_DISK_GET_DRIVE_GEOMETRY_EX, &[], std::mem::size_of::<DISK_GEOMETRY_EX>() + 256);
        let (size, sector_size) = match geo {
            Some(g) if g.len() >= 32 => (
                Some(u64::from_le_bytes(g[24..32].try_into().unwrap())),
                Some(u32::from_le_bytes(g[20..24].try_into().unwrap()) as u64),
            ),
            _ => (None, None),
        };
        let name = format!("PhysicalDrive{n}");
        out.push(DriveInfo {
            path,
            model: desc.as_ref().and_then(|d| d.model.clone()),
            serial: desc.as_ref().and_then(|d| d.serial.clone()),
            size,
            sector_size,
            bus: desc.as_ref().map_or("other", |d| d.bus),
            removable: desc.as_ref().is_some_and(|d| d.removable),
            system: system_disks.contains(&name),
        });
    }
    Ok(out)
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct DeviceDescriptor {
    pub model: Option<String>,
    pub serial: Option<String>,
    pub bus: &'static str,
    pub removable: bool,
}

/// Parses a STORAGE_DEVICE_DESCRIPTOR (strings are offsets into the buffer).
pub(crate) fn parse_device_descriptor(d: &[u8]) -> Option<DeviceDescriptor> {
    if d.len() < 36 {
        return None;
    }
    let u32_at = |o: usize| u32::from_le_bytes(d[o..o + 4].try_into().unwrap()) as usize;
    let string_at = |o: usize| {
        let off = u32_at(o);
        if off == 0 || off >= d.len() {
            return None;
        }
        let end = d[off..].iter().position(|&b| b == 0).map_or(d.len(), |i| off + i);
        // NVMe serials come back with a trailing '.' as a terminator.
        let s = String::from_utf8_lossy(&d[off..end]).trim().trim_end_matches('.').trim().to_string();
        (!s.is_empty()).then_some(s)
    };
    let vendor = string_at(12);
    let product = string_at(16);
    let model = match (vendor, product) {
        (Some(v), Some(p)) if !p.starts_with(&v) && v != "ATA" => Some(format!("{v} {p}")),
        (_, Some(p)) => Some(p),
        (v, None) => v,
    };
    let bus = match u32_at(28) {
        0x01 | 0x0A => "scsi", // SCSI, SAS
        0x02 | 0x03 | 0x0B => "sata", // ATAPI, ATA, SATA
        0x07 => "usb",
        0x0C | 0x0D => "sd", // SD, MMC
        0x0E..=0x10 => "virtual", // Virtual, FileBackedVirtual, Spaces
        0x11 => "nvme",
        _ => "other",
    };
    Some(DeviceDescriptor { model, serial: string_at(24), bus, removable: d[10] != 0 })
}

pub fn is_elevated() -> bool {
    unsafe {
        let mut token: HANDLE = null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
            return false;
        }
        let token = Handle(token);
        let mut elev = TOKEN_ELEVATION { TokenIsElevated: 0 };
        let mut len = 0u32;
        let ok = GetTokenInformation(
            token.0,
            TokenElevation,
            &mut elev as *mut _ as *mut _,
            std::mem::size_of::<TOKEN_ELEVATION>() as u32,
            &mut len,
        );
        ok != 0 && elev.TokenIsElevated != 0
    }
}

/// Disk numbers behind a volume handle (more than one for spanned/striped
/// dynamic volumes).
fn volume_disks(volume: &str) -> Result<Vec<String>, String> {
    let h = open_no_access(volume).ok_or_else(|| format!("can't open volume {volume} (error {})", unsafe { GetLastError() }))?;
    let raw = ioctl(&h, IOCTL_VOLUME_GET_VOLUME_DISK_EXTENTS, &[], 8 + 24 * 64)
        .ok_or_else(|| format!("volume {volume} doesn't report which disk it is on (error {})", unsafe { GetLastError() }))?;
    Ok(parse_disk_extents(&raw))
}

/// VOLUME_DISK_EXTENTS: count at 0, then 24-byte DISK_EXTENTs from 8.
pub(crate) fn parse_disk_extents(raw: &[u8]) -> Vec<String> {
    if raw.len() < 8 {
        return Vec::new();
    }
    let n = u32::from_le_bytes(raw[0..4].try_into().unwrap()) as usize;
    let mut disks: Vec<String> = (0..n)
        .filter_map(|i| raw.get(8 + i * 24..8 + i * 24 + 4))
        .map(|b| format!("PhysicalDrive{}", u32::from_le_bytes(b.try_into().unwrap())))
        .collect();
    disks.dedup();
    disks
}

pub fn backing_of_source(source: &str) -> Result<Backing, String> {
    let rest = source.strip_prefix(r"\\.\").ok_or_else(|| format!("{source} is not a raw device path"))?;
    if let Some(n) = rest.strip_prefix("PhysicalDrive").or_else(|| rest.strip_prefix("PHYSICALDRIVE")) {
        let n: u32 = n.parse().map_err(|_| format!("{source} is not a valid physical drive path"))?;
        return Ok(Backing::Disks(vec![format!("PhysicalDrive{n}")]));
    }
    // A volume (\\.\E:, \\.\HarddiskVolume3, \\.\Volume{...}): ask which disk.
    volume_disks(source).map(Backing::Disks)
}

pub fn backing_of_path(existing: &Path) -> Result<Backing, String> {
    let p = existing.as_os_str().encode_wide().chain(std::iter::once(0)).collect::<Vec<u16>>();
    let mut vol_path = vec![0u16; 1024];
    if unsafe { GetVolumePathNameW(p.as_ptr(), vol_path.as_mut_ptr(), vol_path.len() as u32) } == 0 {
        return Err(format!("can't find the volume holding {} (error {})", existing.display(), unsafe { GetLastError() }));
    }
    match unsafe { GetDriveTypeW(vol_path.as_ptr()) } {
        DRIVE_REMOTE | DRIVE_RAMDISK => return Ok(Backing::NotLocal),
        _ => {}
    }
    let mut guid = vec![0u16; 128];
    if unsafe { GetVolumeNameForVolumeMountPointW(vol_path.as_ptr(), guid.as_mut_ptr(), guid.len() as u32) } == 0 {
        return Err(format!("can't identify the volume holding {} (error {})", existing.display(), unsafe { GetLastError() }));
    }
    let len = guid.iter().position(|&c| c == 0).unwrap_or(guid.len());
    // "\\?\Volume{…}\" → open without the trailing backslash (that would
    // open the root folder instead of the volume).
    let name = String::from_utf16_lossy(&guid[..len]);
    let disks = volume_disks(name.trim_end_matches('\\'))?;
    if disks.is_empty() {
        return Err(format!("the volume holding {} reports no disks", existing.display()));
    }
    Ok(Backing::Disks(disks))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_storage_device_descriptor() {
        let mut d = vec![0u8; 128];
        d[10] = 1; // removable
        d[12..16].copy_from_slice(&64u32.to_le_bytes());
        d[16..20].copy_from_slice(&72u32.to_le_bytes());
        d[24..28].copy_from_slice(&96u32.to_le_bytes());
        d[28..32].copy_from_slice(&7u32.to_le_bytes()); // USB
        d[64..71].copy_from_slice(b"SanDisk");
        d[72..78].copy_from_slice(b"Cruzer");
        d[96..106].copy_from_slice(b"  4C530001");
        let p = parse_device_descriptor(&d).unwrap();
        assert_eq!(
            p,
            DeviceDescriptor { model: Some("SanDisk Cruzer".into()), serial: Some("4C530001".into()), bus: "usb", removable: true }
        );
        d[12..16].fill(0);
        d[28..32].copy_from_slice(&0x11u32.to_le_bytes());
        assert_eq!(parse_device_descriptor(&d).unwrap().model.as_deref(), Some("Cruzer"));
        assert_eq!(parse_device_descriptor(&d).unwrap().bus, "nvme");
    }

    #[test]
    fn parses_disk_extents() {
        let mut raw = vec![0u8; 8 + 48];
        raw[0] = 2;
        raw[8] = 1;
        raw[32] = 3;
        assert_eq!(parse_disk_extents(&raw), vec!["PhysicalDrive1", "PhysicalDrive3"]);
    }

    #[test]
    fn source_paths_resolve() {
        assert_eq!(backing_of_source(r"\\.\PhysicalDrive2").unwrap(), Backing::Disks(vec!["PhysicalDrive2".into()]));
        assert!(backing_of_source(r"\\.\PhysicalDriveX").is_err());
    }

    #[test]
    fn system_drive_resolves_unelevated() {
        let d = std::env::var("SystemDrive").unwrap();
        match backing_of_path(Path::new(&format!("{d}\\"))).unwrap() {
            Backing::Disks(disks) => assert!(disks[0].starts_with("PhysicalDrive")),
            Backing::NotLocal => panic!("system drive reported as not local"),
        }
    }
}
