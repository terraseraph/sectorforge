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
