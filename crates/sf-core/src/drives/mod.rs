//! Physical drive listing (`devices`) and the "is this output on the source
//! drive?" check that stops a recovery from writing onto the failing disk
//! it is rescuing.

use std::io;
use std::path::{Path, PathBuf};

#[cfg(target_os = "linux")]
mod linux;
#[cfg(windows)]
mod windows;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DriveInfo {
    pub path: String,
    pub model: Option<String>,
    pub serial: Option<String>,
    pub size: Option<u64>,
    pub sector_size: Option<u64>,
    /// "sata", "nvme", "usb", "scsi", "sd", "virtual" or "other".
    pub bus: &'static str,
    pub removable: bool,
    pub system: bool,
}

/// Which physical disks back a path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Backing {
    /// Disk names: `PhysicalDrive1` on Windows, `sda` on Linux.
    Disks(Vec<String>),
    /// A network share, RAM disk, etc.: not on any local physical disk.
    NotLocal,
}

pub fn list() -> io::Result<Vec<DriveInfo>> {
    #[cfg(windows)]
    return windows::list();
    #[cfg(target_os = "linux")]
    return linux::list();
    #[cfg(not(any(windows, target_os = "linux")))]
    return Ok(Vec::new());
}

/// Whether this process can open drives for reading.
pub fn is_elevated() -> bool {
    #[cfg(windows)]
    return windows::is_elevated();
    #[cfg(target_os = "linux")]
    return linux::is_elevated();
    #[cfg(not(any(windows, target_os = "linux")))]
    return false;
}

/// The physical disks behind a raw source path (`\\.\PhysicalDrive1`,
/// `\\.\C:`, `/dev/sdb`, `/dev/sdb1`).
pub fn backing_of_source(source: &str) -> Result<Backing, String> {
    #[cfg(windows)]
    return windows::backing_of_source(source);
    #[cfg(target_os = "linux")]
    return linux::backing_of_source(source);
    #[cfg(not(any(windows, target_os = "linux")))]
    return Err(format!("can't tell which disk {source} is on this platform"));
}

/// The physical disks an output file or folder (which may not exist yet)
/// would be written to.
pub fn backing_of_output(path: &Path) -> Result<Backing, String> {
    let existing = existing_ancestor(path).map_err(|e| format!("can't resolve {}: {e}", path.display()))?;
    #[cfg(windows)]
    return windows::backing_of_path(&existing);
    #[cfg(target_os = "linux")]
    return linux::backing_of_path(&existing);
    #[cfg(not(any(windows, target_os = "linux")))]
    return Err(format!("can't tell which disk {} is on this platform", existing.display()));
}

/// Refuses (with a plain-English reason) if any output lives on the same
/// physical disk as the source, or if that can't be determined: guessing
/// wrong here means writing onto the failing drive.
pub fn ensure_outputs_off_source(source: &str, outputs: &[(&str, &Path)]) -> Result<(), String> {
    let src = match backing_of_source(source)? {
        Backing::Disks(d) => d,
        Backing::NotLocal => return Ok(()),
    };
    for (what, path) in outputs {
        let out = backing_of_output(path).map_err(|e| {
            format!(
                "refusing to start: can't tell which drive {what} {} is on ({e}). Put it on a different, healthy drive \
                 (a normal drive letter or mount point).",
                path.display()
            )
        })?;
        if let Backing::Disks(disks) = out {
            if let Some(d) = disks.iter().find(|d| src.contains(d)) {
                return Err(format!(
                    "refusing to start: {what} {} is on the source drive ({d}). Writing to a failing drive is how data gets \
                     destroyed. Choose a location on a different drive.",
                    path.display()
                ));
            }
        }
    }
    Ok(())
}

/// The nearest ancestor of `path` (made absolute) that exists.
fn existing_ancestor(path: &Path) -> io::Result<PathBuf> {
    let abs = std::path::absolute(path)?;
    let mut cur: &Path = &abs;
    loop {
        if cur.exists() {
            return std::fs::canonicalize(cur);
        }
        cur = cur.parent().ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no existing parent folder"))?;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn existing_ancestor_walks_up_to_a_real_folder() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("not").join("yet").join("there.img");
        assert_eq!(existing_ancestor(&p).unwrap(), std::fs::canonicalize(dir.path()).unwrap());
    }

    #[test]
    fn temp_folder_resolves_to_a_local_disk() {
        // Whatever disk the temp folder is on, it must resolve to at least
        // one physical disk (a ramdisk/tmpfs counts as not local).
        let dir = tempfile::tempdir().unwrap();
        match backing_of_output(&dir.path().join("x")) {
            Ok(Backing::Disks(d)) => assert!(!d.is_empty()),
            Ok(Backing::NotLocal) => {}
            Err(e) => panic!("could not resolve temp dir: {e}"),
        }
    }

    #[test]
    fn output_on_the_source_disk_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let Ok(Backing::Disks(d)) = backing_of_output(dir.path()) else { return };
        #[cfg(windows)]
        let source = format!(r"\\.\{}", d[0]);
        #[cfg(not(windows))]
        let source = format!("/dev/{}", d[0]);
        let err = ensure_outputs_off_source(&source, &[("--dest", &dir.path().join("x.img"))]).unwrap_err();
        assert!(err.contains("source drive"), "{err}");
    }
}
