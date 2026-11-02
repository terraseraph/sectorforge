//! NTFS file listing and recovery straight from the MFT, including deleted
//! files. Built to be read off damaged disks: torn records are skipped,
//! broken parent chains become orphans instead of wrong paths, and every
//! file says honestly how much of its data is still there.

mod extract;
mod record;
mod runlist;
mod volume;

#[cfg(test)]
mod tests;

use std::collections::HashMap;

pub use extract::{extract, ExtractProgress, Extracted, Volume};
pub(crate) use extract::{unreadable_note, REUSED_NOTE};
pub use record::{record_number, ref_seq};
pub use runlist::{decode_runlist, Run};
pub use volume::{parse_boot, Boot, DataStream, FileEntry, Ntfs};

pub(crate) const ROOT: u64 = 5;
const EXTEND: u64 = 11;
/// Records 0–15 are NTFS's own metadata files ($MFT, $LogFile, $Bitmap…).
pub(crate) const FIRST_USER_RECORD: u64 = 16;
const BATCH: u64 = 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Recoverable {
    Full,
    Partial,
    None,
}

#[derive(Debug, Clone)]
pub struct IndexEntry {
    pub seq: u16,
    pub in_use: bool,
    pub is_dir: bool,
    pub parent_ref: u64,
    pub name: String,
    pub size: u64,
    pub created: u64,
    pub modified: u64,
    pub recoverable: Recoverable,
}

/// Where a record sits in the tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ancestry {
    /// Ids from just below the root down to the record itself.
    Rooted(Vec<u64>),
    /// The parent chain breaks: ids from the topmost record whose parent is
    /// gone (or reused) down to the record itself.
    Orphaned(Vec<u64>),
}

/// Every named base record on the volume, with what's needed to build
/// paths and judge recoverability.
pub struct Index {
    entries: HashMap<u64, IndexEntry>,
    pub records_total: u64,
    /// `$Bitmap`, kept to judge whether a deleted file's clusters were reused.
    pub(crate) bitmap: Vec<u8>,
}

impl Index {
    /// Reads the whole MFT. `progress(records_read, records_total)` is called
    /// after each batch.
    pub fn build(ntfs: &mut Ntfs, mut progress: impl FnMut(u64, u64)) -> Index {
        let bitmap = ntfs.cluster_bitmap();
        let total = ntfs.record_count();
        let mut entries = HashMap::new();
        let mut first = 0;
        while first < total {
            let n = BATCH.min(total - first);
            let mut recs = Vec::new();
            ntfs.for_each_record(first, n, |id, r| recs.push((id, r)));
            for (id, rec) in recs {
                if rec.base_ref != 0 {
                    continue; // extension record, merged into its base
                }
                let e = ntfs.entry_from(id, rec);
                let Some(name) = e.name.clone() else { continue };
                let recoverable = assess(ntfs, &e, &bitmap);
                entries.insert(
                    id,
                    IndexEntry {
                        seq: e.seq,
                        in_use: e.in_use,
                        is_dir: e.is_dir,
                        parent_ref: name.parent_ref,
                        name: name.name,
                        size: if e.is_dir { 0 } else { e.data.as_ref().map_or(0, |d| d.size) },
                        created: e.created,
                        modified: e.modified,
                        recoverable,
                    },
                );
            }
            first += n;
            progress(first, total);
        }
        Index { entries, records_total: total, bitmap }
    }

    /// For the other filesystems' readers, which number their entries the
    /// NTFS way: the root is [`ROOT`], everything else from
    /// [`FIRST_USER_RECORD`] up, `parent_ref` is the parent's id with
    /// sequence 0.
    pub(crate) fn from_entries(entries: HashMap<u64, IndexEntry>) -> Index {
        Index { records_total: entries.len() as u64, entries, bitmap: Vec::new() }
    }

    pub fn get(&self, id: u64) -> Option<&IndexEntry> {
        self.entries.get(&id)
    }

    /// All ids, ascending.
    pub fn ids(&self) -> Vec<u64> {
        let mut ids: Vec<u64> = self.entries.keys().copied().collect();
        ids.sort_unstable();
        ids
    }

    /// The parent's id, if the parent is still the directory this record
    /// was created in. Deleting a record bumps its sequence number, so a
    /// deleted parent may be exactly one ahead; anything else means the
    /// parent record has been reused for something else.
    pub fn parent(&self, id: u64) -> Option<u64> {
        let e = self.entries.get(&id)?;
        let pid = record_number(e.parent_ref);
        let pseq = ref_seq(e.parent_ref);
        let p = self.entries.get(&pid)?;
        let same = p.seq == pseq || (!p.in_use && p.seq == pseq.wrapping_add(1));
        (p.is_dir && same && pid != id).then_some(pid)
    }

    pub fn ancestry(&self, id: u64) -> Ancestry {
        let mut chain = vec![];
        let mut cur = id;
        loop {
            if cur == ROOT {
                chain.reverse();
                return Ancestry::Rooted(chain);
            }
            chain.push(cur);
            match self.parent(cur) {
                Some(p) if chain.len() < 4096 && !chain.contains(&p) => cur = p,
                _ => {
                    chain.reverse();
                    return Ancestry::Orphaned(chain);
                }
            }
        }
    }

    /// `/`-separated path from the volume root, or `None` for an orphan.
    pub fn path(&self, id: u64) -> Option<String> {
        if id == ROOT {
            return Some("/".into());
        }
        match self.ancestry(id) {
            Ancestry::Rooted(chain) => {
                let mut p = String::new();
                for i in chain {
                    p.push('/');
                    p.push_str(&self.entries[&i].name);
                }
                Some(p)
            }
            Ancestry::Orphaned(_) => None,
        }
    }

    /// User-visible: not the root, not an NTFS metadata file, not inside
    /// `$Extend` (journals, quotas, object ids).
    pub fn is_listed(&self, id: u64) -> bool {
        if id < FIRST_USER_RECORD || !self.entries.contains_key(&id) {
            return false;
        }
        match self.ancestry(id) {
            Ancestry::Rooted(chain) | Ancestry::Orphaned(chain) => !chain.contains(&EXTEND),
        }
    }

    pub fn children(&self) -> HashMap<u64, Vec<u64>> {
        let mut map: HashMap<u64, Vec<u64>> = HashMap::new();
        for id in self.ids() {
            if let Some(p) = self.parent(id) {
                map.entry(p).or_default().push(id);
            }
        }
        map
    }
}

fn assess(ntfs: &Ntfs, e: &FileEntry, bitmap: &[u8]) -> Recoverable {
    if e.is_dir {
        return Recoverable::Full;
    }
    let Some(ds) = &e.data else { return Recoverable::Full };
    if ds.size == 0 {
        return if ds.incomplete { Recoverable::Partial } else { Recoverable::Full };
    }
    if ds.compressed || ds.encrypted {
        return Recoverable::Partial;
    }
    if ds.resident.is_some() {
        return Recoverable::Full;
    }
    let (total, unreadable, reused) = ntfs.damage(ds, (!e.in_use).then_some(bitmap));
    let lost = (unreadable + reused).min(total);
    if total > 0 && lost >= total {
        Recoverable::None
    } else if lost > 0 || ds.incomplete {
        Recoverable::Partial
    } else {
        Recoverable::Full
    }
}
