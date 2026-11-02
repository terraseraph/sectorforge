//! Exercises `LinuxBlockDevice` (O_DIRECT, BLKGETSIZE64/BLKSSZGET) against a
//! real loop device. Needs one set up first, so it's ignored by default:
//! run `scripts/linux-loopdev-test.sh`, which creates the device and passes
//! it in via SF_LOOPDEV / SF_LOOPDEV_BACKING.
#![cfg(target_os = "linux")]

use sf_core::device::LinuxBlockDevice;
use sf_core::drives::{backing_of_source, Backing};
use sf_core::reader::{RangeSet, SafeReader};
use sf_core::{BlockDevice, Imager, ImagerConfig, MapFile, Status};

#[test]
#[ignore = "needs a loop device: run scripts/linux-loopdev-test.sh"]
fn reads_a_loop_device_with_direct_io() {
    let dev_path = std::env::var("SF_LOOPDEV").expect("SF_LOOPDEV not set");
    let backing = std::fs::read(std::env::var("SF_LOOPDEV_BACKING").expect("SF_LOOPDEV_BACKING not set")).unwrap();
    let want_sector: u64 = std::env::var("SF_LOOPDEV_SECTOR").map_or(512, |s| s.parse().unwrap());

    let mut dev = LinuxBlockDevice::open(&dev_path).unwrap();
    assert_eq!(dev.size_bytes(), backing.len() as u64);
    assert_eq!(dev.sector_size(), want_sector);

    // Unaligned reads go through the aligned SafeReader path.
    {
        let mut r = SafeReader::new(&mut dev, RangeSet::new());
        let mut buf = vec![0u8; 10_000];
        assert_eq!(r.read(12_345, &mut buf), 0);
        assert_eq!(&buf[..], &backing[12_345..22_345]);
    }

    // Full image through the imager, compared byte for byte.
    let dest = tempfile::NamedTempFile::new().unwrap();
    let file = std::fs::OpenOptions::new().write(true).open(dest.path()).unwrap();
    file.set_len(backing.len() as u64).unwrap();
    let map = MapFile::new_non_tried(backing.len() as u64);
    let mut imager = Imager::new(dev, file, map, ImagerConfig::default());
    imager.run_copy_pass(|_| {}).unwrap();
    imager.run_retry_pass(|_| {}).unwrap();
    assert_eq!(imager.map().bytes_with_status(Status::Finished), backing.len() as u64);
    drop(imager);
    assert!(std::fs::read(dest.path()).unwrap() == backing, "image differs from loop device contents");

    let name = dev_path.trim_start_matches("/dev/").to_string();
    assert_eq!(backing_of_source(&dev_path).unwrap(), Backing::Disks(vec![name]));
}
