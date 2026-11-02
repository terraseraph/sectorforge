//! Copying files out of an NTFS volume by MFT id, recreating folders under
//! the output directory. Names come off a possibly damaged disk, so every
//! component is sanitized, nothing existing is overwritten, and every
//! directory is checked to still be inside `out` before writing to it.

use std::collections::{HashMap, HashSet};
use std::fs::{self, File};
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};

use super::{Ancestry, Index, Ntfs, ROOT};
use crate::sanitize::{safe_component, with_id_suffix};

const ORPHANS: &str = "_orphaned";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Extracted {
    pub id: u64,
    /// Relative to `out`, `/`-separated. `None` if nothing could be written.
    pub path: Option<String>,
    pub size: u64,
    pub complete: bool,
    pub note: Option<String>,
}

#[derive(Debug, Clone, Copy)]
pub struct ExtractProgress {
    pub files_done: u64,
    pub files_total: u64,
    pub bytes_done: u64,
    pub bytes_total: u64,
}

/// Extracts `ids` (directories recursively) into `out`, calling `on_item`
/// once per id. Returns (extracted, failed).
pub fn extract(
    ntfs: &mut Ntfs,
    index: &Index,
    ids: &[u64],
    out: &Path,
    mut on_item: impl FnMut(&Extracted),
    mut on_progress: impl FnMut(ExtractProgress),
) -> io::Result<(u64, u64)> {
    fs::create_dir_all(out)?;
    let out = fs::canonicalize(out)?;
    let plan = plan(index, ids);
    let mut progress = ExtractProgress {
        files_done: 0,
        files_total: plan.len() as u64,
        bytes_done: 0,
        bytes_total: plan.iter().filter_map(|id| index.get(*id)).map(|e| e.size).sum(),
    };
    let mut w = Writer { out: out.clone(), dirs: HashMap::new(), written: HashSet::new() };
    let (mut ok, mut failed) = (0, 0);

    for id in plan {
        let item = match w.extract_one(ntfs, index, id) {
            Ok(item) => item,
            Err(e) => Extracted { id, path: None, size: 0, complete: false, note: Some(e) },
        };
        if item.path.is_some() {
            ok += 1;
        } else {
            failed += 1;
        }
        progress.files_done += 1;
        progress.bytes_done += index.get(id).map_or(0, |e| e.size);
        on_item(&item);
        on_progress(progress);
    }
    Ok((ok, failed))
}

/// Selected ids plus everything under selected directories, parents first,
/// each once. Unknown ids are kept so they get reported as failures.
fn plan(index: &Index, ids: &[u64]) -> Vec<u64> {
    let children = index.children();
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for &id in ids {
        let mut stack = vec![id];
        while let Some(cur) = stack.pop() {
            if !seen.insert(cur) {
                continue;
            }
            if cur != ROOT {
                out.push(cur);
            }
            if let Some(kids) = children.get(&cur) {
                // NTFS metadata ($MFT, $Extend…) is never pulled in implicitly.
                stack.extend(kids.iter().rev().filter(|k| index.is_listed(**k)));
            }
        }
    }
    out
}

struct Writer {
    out: PathBuf,
    /// Where each directory id ended up in this run.
    dirs: HashMap<u64, PathBuf>,
    /// Files created in this run (never overwritten by another id).
    written: HashSet<PathBuf>,
}

impl Writer {
    fn extract_one(&mut self, ntfs: &mut Ntfs, index: &Index, id: u64) -> Result<Extracted, String> {
        let e = index.get(id).ok_or_else(|| format!("there is no file or folder with id {id} on this volume"))?;
        if e.is_dir {
            let dir = self.dir_for(index, id).map_err(|err| format!("could not create the folder: {err}"))?;
            return Ok(Extracted { id, path: Some(self.rel(&dir)), size: 0, complete: true, note: None });
        }

        let parent = match index.ancestry(id) {
            Ancestry::Rooted(chain) | Ancestry::Orphaned(chain) if chain.len() > 1 => {
                self.dir_for(index, chain[chain.len() - 2])
            }
            Ancestry::Rooted(_) => Ok(self.out.clone()),
            Ancestry::Orphaned(_) => self.orphan_base(),
        }
        .map_err(|err| format!("could not create the folder for it: {err}"))?;

        let name = self.top_name(index, id);
        let mut target = parent.join(&name);
        if target.exists() || self.written.contains(&target) {
            target = parent.join(with_id_suffix(&name, id));
            if self.written.contains(&target) || target.is_dir() {
                return Err("an output file with the same name already exists".into());
            }
        }

        let entry = ntfs.file(id).ok_or("its MFT record can no longer be read")?;
        let ds = entry.data.clone().unwrap_or_default();
        let mut notes = Vec::new();
        let mut file = BufWriter::new(File::create(&target).map_err(|err| format!("could not create the output file: {err}"))?);
        self.written.insert(target.clone());

        let mut size = 0;
        if ds.compressed {
            notes.push("This file is NTFS-compressed, which SectorForge can't decompress yet, so it was left empty.".to_string());
        } else if ds.encrypted {
            notes.push(
                "This file is EFS-encrypted; it can't be decrypted without the original Windows account's key, so it was left empty."
                    .to_string(),
            );
        } else {
            let (_, _, reused) = ntfs.damage(&ds, (!entry.in_use).then_some(&index.bitmap[..]));
            let (written, unreadable) = ntfs.copy_stream(&ds, &mut file).map_err(|err| format!("writing failed: {err}"))?;
            size = written;
            if unreadable > 0 {
                notes.push(format!(
                    "{unreadable} bytes could not be read (bad sectors, not yet imaged, or missing allocation info) and are filled with zeros."
                ));
            }
            if ds.incomplete {
                notes.push("Part of this file's allocation record is missing or damaged.".to_string());
            }
            if reused > 0 {
                notes.push("Some of this deleted file's space has since been reused by other files, so parts of it contain other data.".to_string());
            }
        }
        file.flush().map_err(|err| format!("writing failed: {err}"))?;
        Ok(Extracted {
            id,
            path: Some(self.rel(&target)),
            size,
            complete: notes.is_empty(),
            note: (!notes.is_empty()).then(|| notes.join(" ")),
        })
    }

    /// The sanitized name for `id` (orphan roots also get their id, so two
    /// orphaned "Documents" folders don't merge).
    fn top_name(&self, index: &Index, id: u64) -> String {
        let name = safe_component(&index.get(id).map_or(String::new(), |e| e.name.clone()));
        match index.ancestry(id) {
            Ancestry::Orphaned(chain) if chain.first() == Some(&id) => with_id_suffix(&name, id),
            _ => name,
        }
    }

    fn orphan_base(&mut self) -> io::Result<PathBuf> {
        let p = self.out.join(ORPHANS);
        self.make_dir(&p)?;
        Ok(p)
    }

    /// The output folder for directory `id`, creating it and its parents.
    fn dir_for(&mut self, index: &Index, id: u64) -> io::Result<PathBuf> {
        if id == ROOT {
            return Ok(self.out.clone());
        }
        if let Some(p) = self.dirs.get(&id) {
            return Ok(p.clone());
        }
        let parent = match index.ancestry(id) {
            Ancestry::Rooted(chain) | Ancestry::Orphaned(chain) if chain.len() > 1 => {
                self.dir_for(index, chain[chain.len() - 2])?
            }
            Ancestry::Rooted(_) => self.out.clone(),
            Ancestry::Orphaned(_) => self.orphan_base()?,
        };
        let name = self.top_name(index, id);
        let mut p = parent.join(&name);
        if p.exists() && !p.is_dir() {
            p = parent.join(with_id_suffix(&name, id));
        }
        self.make_dir(&p)?;
        self.dirs.insert(id, p.clone());
        Ok(p)
    }

    /// Creates `p` and refuses if it resolves outside `out` (e.g. a
    /// pre-existing junction or symlink inside the output folder).
    fn make_dir(&self, p: &Path) -> io::Result<()> {
        fs::create_dir_all(p)?;
        let real = fs::canonicalize(p)?;
        if !real.starts_with(&self.out) {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!("{} leads outside the output folder", p.display()),
            ));
        }
        Ok(())
    }

    fn rel(&self, p: &Path) -> String {
        let r = p.strip_prefix(&self.out).unwrap_or(p);
        r.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect::<Vec<_>>().join("/")
    }
}
