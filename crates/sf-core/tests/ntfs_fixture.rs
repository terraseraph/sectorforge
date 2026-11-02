//! `files`/`extract` against a volume formatted and populated by Windows'
//! own NTFS driver (see fixtures/make-ntfs-fixture.ps1), with files created
//! and then deleted: hard links overflowing into $ATTRIBUTE_LIST, a
//! fragmented file, a sparse file, a compressed file, and a deleted folder.

use std::io::Read;
use std::path::Path;

use sf_core::device::FileBlockDevice;
use sf_core::ntfs::{extract, Index, Ntfs, Recoverable};
use sf_core::reader::{RangeSet, SafeReader};
use sha2::{Digest, Sha256};

const FIXTURE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/ntfs-small.vhd.gz");
const MANIFEST: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/ntfs-small.manifest.json");

fn unpack(dir: &Path) -> String {
    let mut gz = flate2::read::GzDecoder::new(std::fs::File::open(FIXTURE).expect("fixture missing: run make-ntfs-fixture.ps1"));
    let mut raw = Vec::new();
    gz.read_to_end(&mut raw).unwrap();
    let p = dir.join("ntfs-small.vhd");
    std::fs::write(&p, raw).unwrap();
    p.to_string_lossy().into_owned()
}

fn sha256(b: &[u8]) -> String {
    Sha256::digest(b).iter().map(|x| format!("{x:02x}")).collect()
}

#[test]
fn reads_a_real_windows_ntfs_volume() {
    let tmp = tempfile::tempdir().unwrap();
    let img = unpack(tmp.path());
    let manifest: Vec<serde_json::Value> = serde_json::from_str(std::fs::read_to_string(MANIFEST).unwrap().trim_start_matches('\u{feff}')).unwrap();

    let mut dev = FileBlockDevice::open(&img, 512).unwrap();
    let scan = sf_core::partitions::scan(&mut dev, false, |_, _| {}).unwrap();
    let part = scan.partitions.iter().find(|p| p.filesystem == sf_core::FsKind::Ntfs).expect("no NTFS partition in fixture");
    let mut reader = SafeReader::new(&mut dev, RangeSet::new());
    let mut vol = Ntfs::open(&mut reader, part.start_bytes).unwrap();
    let index = Index::build(&mut vol, |_, _| {});

    let find = |path: &str| -> u64 {
        let ids = index.ids();
        ids.iter().copied().find(|&id| index.path(id).as_deref() == Some(path)).unwrap_or_else(|| {
            // A hard-linked file is listed under whichever of its names NTFS
            // stored first.
            assert!(path.starts_with("/Links/"), "{path} not found");
            ids.into_iter()
                .find(|&id| index.path(id).is_some_and(|p| p.starts_with("/Links/")) && !index.get(id).unwrap().is_dir)
                .expect("no file under /Links")
        })
    };

    let out = tmp.path().join("out");
    let mut ids = Vec::new();
    for m in &manifest {
        let path = m["path"].as_str().unwrap();
        let id = find(path);
        let e = index.get(id).unwrap();
        assert_eq!(!e.in_use, m["deleted"].as_bool().unwrap(), "{path} deleted flag");
        assert_eq!(e.size, m["size"].as_u64().unwrap(), "{path} size");
        if m["note"] == "ntfs-compressed" {
            assert_eq!(e.recoverable, Recoverable::Partial, "{path}");
        } else {
            assert_eq!(e.recoverable, Recoverable::Full, "{path}");
        }
        assert!(e.modified > 0 && e.created > 0, "{path} timestamps");
        ids.push((id, m));
    }

    let links = find("/Links/target.txt");
    let rec = vol.record(links).unwrap();
    assert!(rec.attrs.iter().any(|a| a.typ == 0x20), "hard-link fixture should exercise $ATTRIBUTE_LIST");

    let mut items = Vec::new();
    let id_list: Vec<u64> = ids.iter().map(|(id, _)| *id).collect();
    extract(&mut vol, &index, &id_list, &out, |i| items.push(i.clone()), |_| {}).unwrap();
    for (id, m) in ids {
        let item = items.iter().find(|i| i.id == id).unwrap();
        let path = m["path"].as_str().unwrap();
        if m["note"] == "ntfs-compressed" {
            assert!(!item.complete && item.note.as_deref().unwrap().contains("compressed"), "{path}");
            continue;
        }
        assert!(item.complete, "{path}: {:?}", item.note);
        let got = std::fs::read(out.join(item.path.as_ref().unwrap())).unwrap();
        assert_eq!(sha256(&got), m["sha256"].as_str().unwrap(), "{path} content");
    }
    assert!(out.join("OldStuff/inner/memo.txt").exists(), "deleted folder tree should be recreated");
}
