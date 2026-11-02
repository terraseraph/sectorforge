//! Linux: drives from `/sys/block`, and path → disk via `st_dev` →
//! `/sys/dev/block/MAJ:MIN`, following partitions up to their disk and
//! device-mapper/md devices down through `slaves/` to the real disks.

use std::ffi::CString;
use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

use super::{Backing, DriveInfo};

const SYS: &str = "/sys";

pub fn list() -> std::io::Result<Vec<DriveInfo>> {
    let system = match backing_of_path(Path::new("/")) {
        Ok(Backing::Disks(d)) => d,
        _ => Vec::new(),
    };
    let mut out = Vec::new();
    let mut names: Vec<String> = fs::read_dir(Path::new(SYS).join("block"))?
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| !["loop", "ram", "zram", "dm-", "md", "sr", "fd", "nbd"].iter().any(|p| n.starts_with(p)))
        .collect();
    names.sort();
    for name in names {
        out.push(drive_info(Path::new(SYS), &name, system.contains(&name)));
    }
    Ok(out)
}

pub(crate) fn drive_info(sys: &Path, name: &str, system: bool) -> DriveInfo {
    let dir = sys.join("block").join(name);
    let read = |rel: &str| fs::read_to_string(dir.join(rel)).ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
    let num = |rel: &str| read(rel).and_then(|s| s.parse::<u64>().ok());
    let serial = read("device/serial").or_else(|| read("serial")).or_else(|| vpd_serial(&dir.join("device/vpd_pg80")));
    let model = match (read("device/vendor"), read("device/model")) {
        (Some(v), Some(m)) if v != "ATA" && !m.starts_with(&v) => Some(format!("{v} {m}")),
        (_, m) => m,
    };
    DriveInfo {
        path: format!("/dev/{name}"),
        model,
        serial,
        // /sys/block/X/size is always in 512-byte units.
        size: num("size").map(|s| s * 512),
        sector_size: num("queue/logical_block_size"),
        bus: bus_of(name, &fs::canonicalize(&dir).unwrap_or(dir.clone())),
        removable: num("removable") == Some(1),
        system,
    }
}

/// SCSI VPD page 0x80 (unit serial number): 4-byte header, then ASCII.
fn vpd_serial(p: &Path) -> Option<String> {
    let b = fs::read(p).ok()?;
    let len = u16::from_be_bytes([*b.get(2)?, *b.get(3)?]) as usize;
    let s = String::from_utf8_lossy(b.get(4..4 + len)?).trim().to_string();
    (!s.is_empty()).then_some(s)
}

pub(crate) fn bus_of(name: &str, real: &Path) -> &'static str {
    let p = real.to_string_lossy();
    if p.contains("/usb") {
        "usb"
    } else if name.starts_with("nvme") || p.contains("/nvme") {
        "nvme"
    } else if name.starts_with("mmcblk") || p.contains("/mmc") {
        "sd"
    } else if name.starts_with("vd") || name.starts_with("xvd") || p.contains("/virtio") || p.contains("/virtual/") {
        "virtual"
    } else if p.contains("/ata") {
        "sata"
    } else if name.starts_with("sd") {
        "scsi"
    } else {
        "other"
    }
}

pub fn is_elevated() -> bool {
    if unsafe { libc::geteuid() } == 0 {
        return true;
    }
    // Members of the `disk` group can read drives without being root.
    list()
        .unwrap_or_default()
        .iter()
        .any(|d| CString::new(d.path.as_str()).is_ok_and(|c| unsafe { libc::access(c.as_ptr(), libc::R_OK) } == 0))
}

pub fn backing_of_source(source: &str) -> Result<Backing, String> {
    let real = fs::canonicalize(source).map_err(|e| format!("can't resolve {source}: {e}"))?;
    let name = real.file_name().map(|n| n.to_string_lossy().into_owned()).ok_or("not a device path")?;
    let disks = base_disks(Path::new(SYS), &name);
    if disks.is_empty() {
        return Err(format!("{source} isn't a block device this system knows about"));
    }
    Ok(Backing::Disks(disks))
}

pub fn backing_of_path(existing: &Path) -> Result<Backing, String> {
    let meta = fs::metadata(existing).map_err(|e| format!("{e}"))?;
    let dev = meta.dev();
    let (maj, min) = (libc::major(dev), libc::minor(dev));
    let sysdev = Path::new(SYS).join("dev/block").join(format!("{maj}:{min}"));
    if let Ok(real) = fs::canonicalize(&sysdev) {
        let name = real.file_name().unwrap().to_string_lossy().into_owned();
        let disks = base_disks(Path::new(SYS), &name);
        if !disks.is_empty() {
            return Ok(Backing::Disks(disks));
        }
    }
    // btrfs and some others report an anonymous device number; the mount
    // table still names the real block device.
    let mountinfo = fs::read_to_string("/proc/self/mountinfo").map_err(|e| format!("can't read the mount table: {e}"))?;
    match mount_source(&mountinfo, existing) {
        Some(src) if src.starts_with("/dev/") => backing_of_source(&src),
        Some(_) => Ok(Backing::NotLocal),
        None => Err(format!("{} isn't on any mounted filesystem", existing.display())),
    }
}

/// The disks under block device `name` (as in /sys/class/block/<name>):
/// a partition's parent disk, or for dm/md devices the disks they're built on.
pub(crate) fn base_disks(sys: &Path, name: &str) -> Vec<String> {
    let mut out = Vec::new();
    collect_base_disks(sys, name, &mut out, 0);
    out.sort();
    out.dedup();
    out
}

fn collect_base_disks(sys: &Path, name: &str, out: &mut Vec<String>, depth: u32) {
    if depth > 16 {
        return;
    }
    let dir = sys.join("class/block").join(name);
    let Ok(real) = fs::canonicalize(&dir) else { return };
    if real.join("partition").exists() {
        if let Some(parent) = real.parent().and_then(|p| p.file_name()) {
            collect_base_disks(sys, &parent.to_string_lossy(), out, depth + 1);
        }
        return;
    }
    let slaves: Vec<PathBuf> = fs::read_dir(real.join("slaves"))
        .map(|rd| rd.filter_map(|e| e.ok()).map(|e| e.path()).collect())
        .unwrap_or_default();
    if slaves.is_empty() {
        out.push(name.to_string());
    }
    for s in slaves {
        collect_base_disks(sys, &s.file_name().unwrap().to_string_lossy(), out, depth + 1);
    }
}

/// The mount source for the longest mount point containing `path`.
pub(crate) fn mount_source(mountinfo: &str, path: &Path) -> Option<String> {
    let mut best: Option<(usize, String)> = None;
    for line in mountinfo.lines() {
        let fields: Vec<&str> = line.split(' ').collect();
        let Some(sep) = fields.iter().position(|f| *f == "-") else { continue };
        let (Some(mp), Some(src)) = (fields.get(4), fields.get(sep + 2)) else { continue };
        let mp = unescape(mp);
        if path.starts_with(&mp) && best.as_ref().is_none_or(|(l, _)| mp.len() >= *l) {
            best = Some((mp.len(), unescape(src)));
        }
    }
    best.map(|(_, s)| s)
}

/// mountinfo escapes space, tab, newline and backslash as \ooo.
fn unescape(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'\\' && i + 3 < b.len() && b[i + 1..i + 4].iter().all(|c| (b'0'..=b'7').contains(c)) {
            out.push(u8::from_str_radix(&s[i + 1..i + 4], 8).unwrap_or(b'?'));
            i += 4;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    /// A fake /sys with sda (+sda1), nvme0n1 (+p2), and dm-0 on sda1 + nvme0n1p2.
    fn fake_sys() -> tempfile::TempDir {
        let t = tempfile::tempdir().unwrap();
        let s = t.path();
        let dev = |rel: &str| {
            let p = s.join("devices").join(rel);
            fs::create_dir_all(&p).unwrap();
            p
        };
        let sda = dev("pci0000:00/ata1/host0/target0/block/sda");
        let sda1 = dev("pci0000:00/ata1/host0/target0/block/sda/sda1");
        fs::write(sda1.join("partition"), "1").unwrap();
        let nv = dev("pci0000:00/nvme/nvme0/nvme0n1");
        let nvp = dev("pci0000:00/nvme/nvme0/nvme0n1/nvme0n1p2");
        fs::write(nvp.join("partition"), "2").unwrap();
        let dm = dev("virtual/block/dm-0");
        fs::create_dir_all(dm.join("slaves")).unwrap();
        symlink(&sda1, dm.join("slaves/sda1")).unwrap();
        symlink(&nvp, dm.join("slaves/nvme0n1p2")).unwrap();
        fs::create_dir_all(s.join("class/block")).unwrap();
        for (n, p) in [("sda", &sda), ("sda1", &sda1), ("nvme0n1", &nv), ("nvme0n1p2", &nvp), ("dm-0", &dm)] {
            symlink(p, s.join("class/block").join(n)).unwrap();
        }
        t
    }

    #[test]
    fn resolves_partitions_and_device_mapper_to_disks() {
        let t = fake_sys();
        assert_eq!(base_disks(t.path(), "sda"), vec!["sda"]);
        assert_eq!(base_disks(t.path(), "sda1"), vec!["sda"]);
        assert_eq!(base_disks(t.path(), "dm-0"), vec!["nvme0n1", "sda"]);
        assert!(base_disks(t.path(), "nope").is_empty());
    }

    #[test]
    fn picks_the_longest_mount_point() {
        let mi = "22 1 8:2 / / rw - ext4 /dev/sda2 rw\n\
                  30 22 0:40 / /mnt/my\\040disk rw - btrfs /dev/sdb1 rw\n\
                  31 22 0:41 / /tmp rw - tmpfs tmpfs rw\n";
        assert_eq!(mount_source(mi, Path::new("/home/x")).as_deref(), Some("/dev/sda2"));
        assert_eq!(mount_source(mi, Path::new("/mnt/my disk/out")).as_deref(), Some("/dev/sdb1"));
        assert_eq!(mount_source(mi, Path::new("/tmp/a")).as_deref(), Some("tmpfs"));
    }

    #[test]
    fn classifies_bus_from_sysfs_path() {
        assert_eq!(bus_of("sdb", Path::new("/sys/devices/pci0000:00/0000:00:14.0/usb2/2-1/host6/block/sdb")), "usb");
        assert_eq!(bus_of("sda", Path::new("/sys/devices/pci0000:00/ata1/host0/block/sda")), "sata");
        assert_eq!(bus_of("nvme0n1", Path::new("/sys/devices/pci0000:00/nvme/nvme0/nvme0n1")), "nvme");
        assert_eq!(bus_of("vda", Path::new("/sys/devices/pci0000:00/virtio1/block/vda")), "virtual");
    }
}
