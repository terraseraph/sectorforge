//! Signature-based file recovery from raw sectors (PhotoRec-style). Finds
//! files by their header bytes at sector boundaries, works out where each
//! one really ends from its own structure, and writes it out. Works with no
//! filesystem at all; recovered files have no original names.

mod formats;
mod probe;

use std::fs::{self, File};
use std::io::{self, Write};
use std::path::Path;

pub use formats::{Kind, TypeInfo, TYPES};
use probe::Probe;

use crate::reader::SafeReader;

const SCAN_CHUNK: u64 = 16 * 1024 * 1024;
const HEADER_WINDOW: u64 = formats::HEADER_LEN as u64;
const COPY_CHUNK: usize = 4 * 1024 * 1024;

#[derive(Debug, Clone)]
pub struct CarveOptions {
    /// Type ids to keep (from [`TYPES`]). Empty = all.
    pub types: Vec<String>,
    pub start: u64,
    /// Exclusive; `None` = end of the source.
    pub end: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CarvedFile {
    pub offset: u64,
    pub size: u64,
    pub type_id: &'static str,
    /// Relative to the output directory, `/`-separated.
    pub name: String,
    pub complete: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CarveSummary {
    pub files_found: u64,
    pub bytes_written: u64,
}

/// Checks `--types` against the known ids; `Err` names the unknown one.
pub fn validate_types(types: &[String]) -> Result<(), String> {
    for t in types {
        if !TYPES.iter().any(|i| i.id == t) {
            let known: Vec<&str> = TYPES.iter().map(|i| i.id).collect();
            return Err(format!("unknown file type \"{t}\" (known types: {})", known.join(", ")));
        }
    }
    Ok(())
}

pub fn carve(
    reader: &mut SafeReader,
    out_dir: &Path,
    opts: &CarveOptions,
    mut on_progress: impl FnMut(u64, u64, u64),
    mut on_file: impl FnMut(&CarvedFile),
) -> io::Result<CarveSummary> {
    let wanted = |id: &str| opts.types.is_empty() || opts.types.iter().any(|t| t == id);
    let kinds: Vec<Kind> = Kind::ALL.into_iter().filter(|k| k.type_ids().iter().any(|id| wanted(id))).collect();

    let sector = reader.sector_size().max(512);
    let size = reader.size();
    let end = opts.end.unwrap_or(size).min(size);
    let start = opts.start.div_ceil(sector) * sector;
    let total = end.saturating_sub(start);
    let mut summary = CarveSummary::default();
    let mut pos = start;
    let mut buf = Vec::new();

    while pos < end {
        let chunk = SCAN_CHUNK.min(end - pos);
        buf.resize((chunk + HEADER_WINDOW).min(size - pos) as usize, 0);
        reader.read(pos, &mut buf);
        let mut next = pos + chunk;

        let mut local = 0u64;
        while local < chunk {
            let abs = pos + local;
            let head = &buf[local as usize..(local + HEADER_WINDOW).min(buf.len() as u64) as usize];
            let hit = formats::identify(head).filter(|k| kinds.contains(k));
            if let Some(kind) = hit.filter(|_| !reader.is_unreadable(abs, abs + sector)) {
                let found = carve_one(reader, out_dir, kind, abs, sector, &wanted)?;
                // Never carve into a region a previous file claimed: its
                // insides (thumbnails, embedded zips) aren't separate files.
                let claimed_end = (abs + found.0.max(1)).div_ceil(sector) * sector;
                if let Some(f) = found.1 {
                    summary.files_found += 1;
                    summary.bytes_written += f.size;
                    on_file(&f);
                }
                if claimed_end >= pos + chunk {
                    next = claimed_end;
                    break;
                }
                local = claimed_end - pos;
                continue;
            }
            local += sector;
        }
        pos = next;
        on_progress(pos.min(end) - start, total, summary.files_found);
    }
    Ok(summary)
}

/// Measures and writes one file. Returns the bytes it claims and the file,
/// or `None` for the file when its final type wasn't asked for.
fn carve_one(
    reader: &mut SafeReader,
    out_dir: &Path,
    kind: Kind,
    abs: u64,
    sector: u64,
    wanted: &dyn Fn(&str) -> bool,
) -> io::Result<(u64, Option<CarvedFile>)> {
    let (len, type_id) = {
        let mut probe = Probe::new(reader, abs, kind.search_limit());
        formats::measure(kind, &mut probe)
    };
    let available = reader.size() - abs;
    let found = len.filter(|&l| l > 0 && l <= kind.max_size());
    let (size, mut complete) = match found {
        Some(l) if l <= available => (l, true),
        Some(_) => (available, false), // runs off the end of the source
        None => (unknown_end(reader, abs, kind.unknown_end_size().min(available), sector), false),
    };
    if !wanted(type_id) {
        return Ok((size, None));
    }

    let name = format!("{type_id}/f{:07}.{type_id}", abs / 512);
    let path = out_dir.join(type_id).join(format!("f{:07}.{type_id}", abs / 512));
    fs::create_dir_all(path.parent().unwrap())?;
    let mut file = File::create(&path)?;
    let mut buf = vec![0u8; COPY_CHUNK];
    let mut done = 0u64;
    while done < size {
        let n = (size - done).min(COPY_CHUNK as u64) as usize;
        if reader.read(abs + done, &mut buf[..n]) > 0 {
            complete = false;
        }
        file.write_all(&buf[..n])?;
        done += n as u64;
    }
    file.flush()?;
    complete &= !reader.is_unreadable(abs, abs + size);
    Ok((size, Some(CarvedFile { offset: abs, size, type_id, name, complete })))
}

/// With no end marker, a file runs until the next recognizable header (any
/// type) or `max`, whichever is first.
fn unknown_end(reader: &mut SafeReader, abs: u64, max: u64, sector: u64) -> u64 {
    let mut probe = Probe::new(reader, abs, max);
    let mut rel = sector;
    while rel < max {
        if formats::identify(&probe.get_upto(rel, HEADER_WINDOW as usize)).is_some() {
            return rel;
        }
        rel += sector;
    }
    max
}

#[cfg(test)]
mod tests;
