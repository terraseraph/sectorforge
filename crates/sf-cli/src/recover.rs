//! `devices`, `carve`, `files`, `extract`: thin JSON/text wrappers over
//! sf-core's drive listing, carver and NTFS reader.

use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, ensure};
use serde_json::{json, Value};
use sf_core::carve::{self, CarveOptions, TYPES};
use sf_core::drives;
use sf_core::ntfs::{self, Index, Ntfs, Recoverable};
use sf_core::reader::{unreadable_from_map, RangeSet, SafeReader};
use sf_core::timefmt::filetime_to_rfc3339;
use sf_core::{BlockDevice, FsKind};

use super::{format_bytes, fs_name, is_raw_device_path, mode_name, open_device, Out, Throttle};

/// Device-mode carving reads big chunks of a possibly failing drive; after
/// a failed read, give up on 64 KiB at a time rather than retrying every
/// sector (the imager is the tool for squeezing out every sector).
const CARVE_DEVICE_GRANULARITY: u64 = 64 * 1024;

pub fn run_devices(out: Out) -> anyhow::Result<()> {
    let list = drives::list().map_err(|e| anyhow!("can't list drives: {e}"))?;
    let elevated = drives::is_elevated();
    if out.json {
        let devices: Vec<Value> = list
            .iter()
            .map(|d| {
                json!({
                    "path": d.path, "model": d.model, "serial": d.serial, "size": d.size,
                    "sector_size": d.sector_size, "bus": d.bus, "removable": d.removable, "system": d.system,
                })
            })
            .collect();
        out.event(json!({"event": "devices", "elevated": elevated, "devices": devices}));
    } else {
        if !elevated {
            println!("(not elevated: reading drives needs Administrator / root)");
        }
        for d in &list {
            println!(
                "{:<20} {:>10}  {:<7} {}{}{}",
                d.path,
                d.size.map(format_bytes).unwrap_or_else(|| "?".into()),
                d.bus,
                d.model.as_deref().unwrap_or("(unknown model)"),
                if d.removable { "  [removable]" } else { "" },
                if d.system { "  [SYSTEM]" } else { "" },
            );
        }
    }
    Ok(())
}

/// In device mode, refuses outputs on the source drive.
pub fn guard_outputs(source: &str, outputs: &[(&str, &Path)]) -> anyhow::Result<()> {
    if is_raw_device_path(source) {
        drives::ensure_outputs_off_source(source, outputs).map_err(|e| anyhow!(e))?;
    }
    Ok(())
}

fn load_unreadable(map: Option<&PathBuf>, size: u64) -> anyhow::Result<RangeSet> {
    match map {
        Some(m) => unreadable_from_map(m, size).map_err(|e| anyhow!(e)),
        None => Ok(RangeSet::new()),
    }
}

fn start_event(out: Out, source: &str, dev: &dyn BlockDevice, extra: Value) {
    let mut ev = json!({
        "event": "start",
        "source": source,
        "mode": mode_name(source),
        "size": dev.size_bytes(),
        "sector_size": dev.sector_size(),
    });
    if let (Some(e), Value::Object(x)) = (ev.as_object_mut(), extra) {
        e.extend(x);
    }
    out.event(ev);
}

pub fn run_list_types(out: Out) {
    if out.json {
        let types: Vec<Value> = TYPES.iter().map(|t| json!({"id": t.id, "description": t.description})).collect();
        out.event(json!({"event": "types", "types": types}));
    } else {
        for t in TYPES {
            println!("{:<8} {}", t.id, t.description);
        }
    }
}

pub struct CarveArgs {
    pub source: String,
    pub out_dir: PathBuf,
    pub types: Vec<String>,
    pub start: Option<u64>,
    pub end: Option<u64>,
    pub map: Option<PathBuf>,
}

pub fn run_carve(out: Out, a: CarveArgs) -> anyhow::Result<()> {
    carve::validate_types(&a.types).map_err(|e| anyhow!(e))?;
    guard_outputs(&a.source, &[("--out", &a.out_dir)])?;
    let mut dev = open_device(&a.source)?;
    let size = dev.size_bytes();
    let start = a.start.unwrap_or(0);
    let end = a.end.unwrap_or(size);
    ensure!(start < end, "--start ({start}) must be before --end ({end})");
    ensure!(start < size, "--start ({start}) is past the end of the source ({size} bytes)");
    let bad = load_unreadable(a.map.as_ref(), size)?;
    std::fs::create_dir_all(&a.out_dir).map_err(|e| anyhow!("can't create output folder {}: {e}", a.out_dir.display()))?;
    start_event(out, &a.source, dev.as_ref(), json!({}));

    let mut reader = SafeReader::new(dev.as_mut(), bad);
    if is_raw_device_path(&a.source) {
        reader = reader.with_granularity(CARVE_DEVICE_GRANULARITY);
    }
    let opts = CarveOptions { types: a.types, start, end: Some(end) };
    let mut throttle = Throttle::new();
    let summary = carve::carve(
        &mut reader,
        &a.out_dir,
        &opts,
        |done, total, found| {
            if done == total || throttle.ready() {
                if out.json {
                    out.event(json!({"event": "carve_progress", "bytes_scanned": done, "bytes_total": total, "files_found": found}));
                } else {
                    eprint!("\rScanned {:>6.2}%, {found} files found", 100.0 * done as f64 / total.max(1) as f64);
                }
            }
        },
        |f| {
            if out.json {
                out.event(json!({
                    "event": "file", "offset": f.offset, "size": f.size, "type": f.type_id,
                    "name": f.name, "complete": f.complete,
                }));
            } else {
                eprintln!();
                println!("{}  {}{}", f.name, format_bytes(f.size), if f.complete { "" } else { "  (incomplete)" });
            }
        },
    )?;
    out.event(json!({"event": "done", "files_found": summary.files_found, "bytes_written": summary.bytes_written}));
    if !out.json {
        eprintln!();
        println!("{} files recovered, {} written", summary.files_found, format_bytes(summary.bytes_written));
    }
    Ok(())
}

/// Opens the source and checks there's NTFS at `partition_lba` (512-byte
/// units, as `scan` reports it).
fn open_ntfs_source(source: &str, partition_lba: u64, map: Option<&PathBuf>) -> anyhow::Result<(Box<dyn BlockDevice>, RangeSet, u64)> {
    let mut dev = open_device(source)?;
    let size = dev.size_bytes();
    let part_off = partition_lba.checked_mul(512).filter(|&o| o < size).ok_or_else(|| {
        anyhow!("--partition-lba {partition_lba} is past the end of the source ({size} bytes)")
    })?;
    let bad = load_unreadable(map, size)?;
    let mut boot = vec![0u8; sf_core::partitions::DETECT_WINDOW];
    let mut r = SafeReader::new(dev.as_mut(), bad.clone());
    if r.read(part_off, &mut boot[..512]) > 0 {
        bail!("the boot sector at LBA {partition_lba} can't be read (bad sector, or not yet imaged)");
    }
    r.read(part_off, &mut boot);
    match sf_core::partitions::detect_filesystem_at(&boot) {
        FsKind::Ntfs => {}
        FsKind::Unknown => bail!("files currently supports NTFS only (no recognizable filesystem at LBA {partition_lba})"),
        other => bail!("files currently supports NTFS only (the partition at LBA {partition_lba} is {})", fs_name(other)),
    }
    drop(r);
    Ok((dev, bad, part_off))
}

fn build_index(out: Out, ntfs: &mut Ntfs) -> Index {
    let mut throttle = Throttle::new();
    Index::build(ntfs, |done, total| {
        if done == total || throttle.ready() {
            if out.json {
                out.event(json!({"event": "files_progress", "records_read": done, "records_total": total}));
            } else {
                eprint!("\rReading MFT: {done}/{total} records");
            }
        }
    })
}

fn recoverable_name(r: Recoverable) -> &'static str {
    match r {
        Recoverable::Full => "full",
        Recoverable::Partial => "partial",
        Recoverable::None => "none",
    }
}

pub fn run_files(out: Out, source: &str, partition_lba: u64, deleted_only: bool, map: Option<&PathBuf>) -> anyhow::Result<()> {
    let (mut dev, bad, part_off) = open_ntfs_source(source, partition_lba, map)?;
    start_event(out, source, dev.as_ref(), json!({"partition_lba": partition_lba, "filesystem": "ntfs"}));
    let mut reader = SafeReader::new(dev.as_mut(), bad);
    let mut vol = Ntfs::open(&mut reader, part_off).map_err(|e| anyhow!(e))?;
    let index = build_index(out, &mut vol);
    if !out.json {
        eprintln!();
    }

    let (mut files, mut deleted) = (0u64, 0u64);
    for id in index.ids() {
        if !index.is_listed(id) {
            continue;
        }
        let e = index.get(id).unwrap();
        if deleted_only && e.in_use {
            continue;
        }
        files += 1;
        deleted += !e.in_use as u64;
        let path = index.path(id);
        if out.json {
            out.event(json!({
                "event": "file", "id": id, "parent_id": ntfs::record_number(e.parent_ref), "name": e.name,
                "path": path, "is_dir": e.is_dir, "size": e.size, "deleted": !e.in_use,
                "modified": filetime_to_rfc3339(e.modified), "created": filetime_to_rfc3339(e.created),
                "recoverable": recoverable_name(e.recoverable),
            }));
        } else {
            println!(
                "{:>8}  {}{:<10} {:>10}  {}",
                id,
                if e.in_use { " " } else { "D" },
                if e.is_dir { "<dir>" } else { recoverable_name(e.recoverable) },
                if e.is_dir { String::new() } else { format_bytes(e.size) },
                path.unwrap_or_else(|| format!("<orphaned>/{}", e.name)),
            );
        }
    }
    out.event(json!({"event": "done", "files": files, "deleted": deleted}));
    if !out.json {
        println!("{files} entries, {deleted} deleted");
    }
    Ok(())
}

pub struct ExtractArgs {
    pub source: String,
    pub partition_lba: u64,
    pub ids: Vec<u64>,
    pub ids_file: Option<PathBuf>,
    pub out_dir: PathBuf,
    pub map: Option<PathBuf>,
}

fn read_ids_file(p: &Path) -> anyhow::Result<Vec<u64>> {
    let text = std::fs::read_to_string(p).map_err(|e| anyhow!("can't read --ids-file {}: {e}", p.display()))?;
    text.lines()
        .enumerate()
        .map(|(i, l)| (i, l.trim()))
        .filter(|(_, l)| !l.is_empty())
        .map(|(i, l)| l.parse::<u64>().map_err(|_| anyhow!("--ids-file line {}: \"{l}\" is not a file id", i + 1)))
        .collect()
}

pub fn run_extract(out: Out, a: ExtractArgs) -> anyhow::Result<()> {
    let mut ids = a.ids.clone();
    if let Some(f) = &a.ids_file {
        ids.extend(read_ids_file(f)?);
    }
    ensure!(!ids.is_empty(), "no file ids given to extract");
    guard_outputs(&a.source, &[("--out", &a.out_dir)])?;
    let (mut dev, bad, part_off) = open_ntfs_source(&a.source, a.partition_lba, a.map.as_ref())?;
    std::fs::create_dir_all(&a.out_dir).map_err(|e| anyhow!("can't create output folder {}: {e}", a.out_dir.display()))?;
    start_event(out, &a.source, dev.as_ref(), json!({"partition_lba": a.partition_lba, "filesystem": "ntfs"}));
    let mut reader = SafeReader::new(dev.as_mut(), bad);
    let mut vol = Ntfs::open(&mut reader, part_off).map_err(|e| anyhow!(e))?;
    let index = build_index(out, &mut vol);
    if !out.json {
        eprintln!();
    }

    let mut throttle = Throttle::new();
    let (ok, failed) = ntfs::extract(
        &mut vol,
        &index,
        &ids,
        &a.out_dir,
        |item| {
            if out.json {
                out.event(json!({
                    "event": "extracted", "id": item.id, "path": item.path, "size": item.size,
                    "complete": item.complete, "note": item.note,
                }));
            } else {
                println!(
                    "{:>8}  {}  {}{}",
                    item.id,
                    if item.path.is_none() { "FAILED " } else if item.complete { "ok     " } else { "partial" },
                    item.path.as_deref().unwrap_or(""),
                    item.note.as_ref().map(|n| format!("\n          {n}")).unwrap_or_default(),
                );
            }
        },
        |p| {
            if out.json && (p.files_done == p.files_total || throttle.ready()) {
                out.event(json!({
                    "event": "extract_progress", "files_done": p.files_done, "files_total": p.files_total,
                    "bytes_done": p.bytes_done, "bytes_total": p.bytes_total,
                }));
            }
        },
    )
    .map_err(|e| anyhow!("can't write to {}: {e}", a.out_dir.display()))?;
    out.event(json!({"event": "done", "extracted": ok, "failed": failed}));
    if !out.json {
        println!("{ok} extracted, {failed} failed");
    }
    Ok(())
}
