//! Platform-specific raw block device backends. Each one implements
//! [`crate::BlockDevice`] by opening the real OS device path for direct,
//! unbuffered reads.

#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use windows::WindowsPhysicalDrive;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
pub use linux::LinuxBlockDevice;

mod file_backend;
pub use file_backend::FileBlockDevice;

/// Marks a file sparse, so a write far into it doesn't first make Windows
/// zero-fill everything before it (a reverse rescue writes the end of the
/// image first). Elsewhere a file grown with `set_len` is already sparse.
pub fn make_sparse(file: &std::fs::File) -> std::io::Result<()> {
    #[cfg(windows)]
    return windows::set_sparse(file);
    #[cfg(not(windows))]
    {
        let _ = file;
        Ok(())
    }
}

/// How a physical drive is read. The pass-through modes send fixed READ
/// commands straight to the drive with their own timeout, skipping the OS
/// disk driver and its long internal retries (DMDE's SCSI/ATA I/O modes).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Access {
    /// Ordinary reads through the OS disk driver.
    #[default]
    Os,
    /// SCSI READ(16) pass-through: SATA, SAS and most USB drives.
    Scsi,
    /// ATA READ DMA EXT pass-through: SATA drives on a SATA port.
    Ata,
}
