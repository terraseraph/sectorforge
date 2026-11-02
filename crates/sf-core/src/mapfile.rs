//! The resumable "map" of a cloning run: which byte ranges of the source
//! device are rescued, bad, or not yet tried.
//!
//! The on-disk format is deliberately close to GNU ddrescue's mapfile, so an
//! interrupted run can be resumed later, and the map can be opened in
//! `ddrescueview` for a visual check before SectorForge has its own GUI.

use std::fmt;
use std::io::{self, Write};
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Status {
    /// Never attempted.
    NonTried,
    /// Attempted in a large chunk that failed; not yet split down to find
    /// exactly which sectors are bad.
    NonTrimmed,
    /// Trimmed down to the bad sectors, but not yet scraped with slow,
    /// single-sector retries.
    NonScraped,
    /// Confirmed unreadable after retries.
    BadSector,
    /// Successfully read and copied to the destination.
    Finished,
}

impl Status {
    pub fn to_char(self) -> char {
        match self {
            Status::NonTried => '?',
            Status::NonTrimmed => '*',
            Status::NonScraped => '/',
            Status::BadSector => '-',
            Status::Finished => '+',
        }
    }

    pub fn from_char(c: char) -> Option<Status> {
        match c {
            '?' => Some(Status::NonTried),
            '*' => Some(Status::NonTrimmed),
            '/' => Some(Status::NonScraped),
            '-' => Some(Status::BadSector),
            '+' => Some(Status::Finished),
            _ => None,
        }
    }
}

impl fmt::Display for Status {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_char())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Block {
    pub pos: u64,
    pub size: u64,
    pub status: Status,
}

impl Block {
    pub fn end(&self) -> u64 {
        self.pos + self.size
    }
}

#[derive(Debug, Clone)]
pub struct MapFile {
    size: u64,
    /// Sorted, non-overlapping, contiguous blocks covering `[0, size)`.
    blocks: Vec<Block>,
    pub current_pos: u64,
    pub current_status: Status,
}

#[derive(Debug, thiserror::Error)]
pub enum MapFileError {
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),
    #[error("malformed mapfile at line {line}: {reason}")]
    Parse { line: usize, reason: String },
}

impl MapFile {
    /// A fresh map for a device of `size` bytes: entirely non-tried.
    pub fn new_non_tried(size: u64) -> Self {
        let blocks = if size == 0 {
            Vec::new()
        } else {
            vec![Block {
                pos: 0,
                size,
                status: Status::NonTried,
            }]
        };
        Self {
            size,
            blocks,
            current_pos: 0,
            current_status: Status::NonTried,
        }
    }

    pub fn size(&self) -> u64 {
        self.size
    }

    pub fn blocks(&self) -> &[Block] {
        &self.blocks
    }

    /// Total bytes currently carrying `status`.
    pub fn bytes_with_status(&self, status: Status) -> u64 {
        self.blocks
            .iter()
            .filter(|b| b.status == status)
            .map(|b| b.size)
            .sum()
    }

    /// Sets `[start, start+len)` to `status`, splitting and re-merging
    /// blocks as needed. Panics if the range runs past the device size.
    pub fn mark(&mut self, start: u64, len: u64, status: Status) {
        if len == 0 {
            return;
        }
        let end = start + len;
        assert!(end <= self.size, "mark() range runs past device size");

        let mut new_blocks = Vec::with_capacity(self.blocks.len() + 2);
        for b in &self.blocks {
            let b_end = b.end();
            if b_end <= start || b.pos >= end {
                new_blocks.push(*b);
                continue;
            }
            if b.pos < start {
                new_blocks.push(Block {
                    pos: b.pos,
                    size: start - b.pos,
                    status: b.status,
                });
            }
            let ov_start = b.pos.max(start);
            let ov_end = b_end.min(end);
            new_blocks.push(Block {
                pos: ov_start,
                size: ov_end - ov_start,
                status,
            });
            if b_end > end {
                new_blocks.push(Block {
                    pos: end,
                    size: b_end - end,
                    status: b.status,
                });
            }
        }
        new_blocks.sort_by_key(|b| b.pos);
        self.blocks = merge_adjacent(new_blocks);
    }

    /// The first block at or after `from` whose status is in `statuses`,
    /// clipped so it never starts before `from`. Returns `(pos, len)`.
    pub fn find_next(&self, from: u64, statuses: &[Status]) -> Option<(u64, u64)> {
        for b in &self.blocks {
            if b.end() <= from {
                continue;
            }
            if statuses.contains(&b.status) {
                let pos = b.pos.max(from);
                return Some((pos, b.end() - pos));
            }
        }
        None
    }

    pub fn to_ddrescue_string(&self) -> String {
        let mut out = String::new();
        out.push_str("# Mapfile. Created by SectorForge\n");
        out.push_str("# current_pos  current_status\n");
        out.push_str(&format!(
            "0x{:X}     {}\n",
            self.current_pos,
            self.current_status.to_char()
        ));
        out.push_str("#      pos        size  status\n");
        for b in &self.blocks {
            out.push_str(&format!(
                "0x{:08X}  0x{:08X}  {}\n",
                b.pos,
                b.size,
                b.status.to_char()
            ));
        }
        out
    }

    pub fn save(&self, path: impl AsRef<Path>) -> io::Result<()> {
        let tmp_path = {
            let mut p = path.as_ref().to_path_buf();
            let mut name = p.file_name().unwrap_or_default().to_os_string();
            name.push(".tmp");
            p.set_file_name(name);
            p
        };
        {
            let mut f = std::fs::File::create(&tmp_path)?;
            f.write_all(self.to_ddrescue_string().as_bytes())?;
            f.sync_all()?;
        }
        // Replacing a file another process has open briefly fails on
        // Windows; retry before giving up.
        let mut attempt = 0;
        loop {
            match std::fs::rename(&tmp_path, path.as_ref()) {
                Ok(()) => return Ok(()),
                Err(e) if attempt < 5 && e.kind() == io::ErrorKind::PermissionDenied => {
                    attempt += 1;
                    std::thread::sleep(std::time::Duration::from_millis(50));
                }
                Err(e) => return Err(e),
            }
        }
    }

    pub fn load(path: impl AsRef<Path>) -> Result<Self, MapFileError> {
        let text = std::fs::read_to_string(path)?;
        Self::from_ddrescue_str(&text)
    }

    pub fn from_ddrescue_str(text: &str) -> Result<Self, MapFileError> {
        let mut current_pos = 0u64;
        let mut current_status = Status::NonTried;
        let mut blocks = Vec::new();
        let mut seen_header_line = false;

        for (i, raw_line) in text.lines().enumerate() {
            let line = raw_line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let fields: Vec<&str> = line.split_whitespace().collect();
            if !seen_header_line {
                // current_pos  current_status
                if fields.len() < 2 {
                    return Err(MapFileError::Parse {
                        line: i + 1,
                        reason: "expected current_pos/current_status line".into(),
                    });
                }
                current_pos = parse_hex_u64(fields[0], i + 1)?;
                current_status = Status::from_char(fields[1].chars().next().ok_or_else(|| {
                    MapFileError::Parse {
                        line: i + 1,
                        reason: "empty status field".into(),
                    }
                })?)
                .ok_or_else(|| MapFileError::Parse {
                    line: i + 1,
                    reason: format!("unknown status char {:?}", fields[1]),
                })?;
                seen_header_line = true;
                continue;
            }
            // pos  size  status
            if fields.len() < 3 {
                return Err(MapFileError::Parse {
                    line: i + 1,
                    reason: "expected pos/size/status block line".into(),
                });
            }
            let pos = parse_hex_u64(fields[0], i + 1)?;
            let size = parse_hex_u64(fields[1], i + 1)?;
            let status = Status::from_char(fields[2].chars().next().ok_or_else(|| {
                MapFileError::Parse {
                    line: i + 1,
                    reason: "empty status field".into(),
                }
            })?)
            .ok_or_else(|| MapFileError::Parse {
                line: i + 1,
                reason: format!("unknown status char {:?}", fields[2]),
            })?;
            blocks.push(Block { pos, size, status });
        }

        blocks.sort_by_key(|b| b.pos);
        let size = blocks.last().map(|b| b.end()).unwrap_or(0);

        Ok(Self {
            size,
            blocks: merge_adjacent(blocks),
            current_pos,
            current_status,
        })
    }
}

fn parse_hex_u64(field: &str, line: usize) -> Result<u64, MapFileError> {
    let stripped = field.strip_prefix("0x").or_else(|| field.strip_prefix("0X")).unwrap_or(field);
    u64::from_str_radix(stripped, 16).map_err(|e| MapFileError::Parse {
        line,
        reason: format!("invalid hex number {:?}: {e}", field),
    })
}

fn merge_adjacent(blocks: Vec<Block>) -> Vec<Block> {
    let mut out: Vec<Block> = Vec::with_capacity(blocks.len());
    for b in blocks {
        if let Some(last) = out.last_mut() {
            if last.status == b.status && last.end() == b.pos {
                last.size += b.size;
                continue;
            }
        }
        out.push(b);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fresh_map_is_one_non_tried_block() {
        let m = MapFile::new_non_tried(1000);
        assert_eq!(m.blocks().len(), 1);
        assert_eq!(m.blocks()[0], Block { pos: 0, size: 1000, status: Status::NonTried });
    }

    #[test]
    fn mark_splits_and_merges() {
        let mut m = MapFile::new_non_tried(1000);
        m.mark(100, 200, Status::Finished);
        assert_eq!(
            m.blocks(),
            &[
                Block { pos: 0, size: 100, status: Status::NonTried },
                Block { pos: 100, size: 200, status: Status::Finished },
                Block { pos: 300, size: 700, status: Status::NonTried },
            ]
        );

        // Adjacent finished range merges into one block.
        m.mark(300, 100, Status::Finished);
        assert_eq!(
            m.blocks(),
            &[
                Block { pos: 0, size: 100, status: Status::NonTried },
                Block { pos: 100, size: 300, status: Status::Finished },
                Block { pos: 400, size: 600, status: Status::NonTried },
            ]
        );
    }

    #[test]
    fn mark_can_carve_a_hole_out_of_a_larger_block() {
        let mut m = MapFile::new_non_tried(1000);
        m.mark(400, 100, Status::BadSector);
        assert_eq!(
            m.blocks(),
            &[
                Block { pos: 0, size: 400, status: Status::NonTried },
                Block { pos: 400, size: 100, status: Status::BadSector },
                Block { pos: 500, size: 500, status: Status::NonTried },
            ]
        );
    }

    #[test]
    fn find_next_skips_finished_and_clips_to_from() {
        let mut m = MapFile::new_non_tried(1000);
        m.mark(0, 500, Status::Finished);
        assert_eq!(m.find_next(0, &[Status::NonTried]), Some((500, 500)));
        assert_eq!(m.find_next(700, &[Status::NonTried]), Some((700, 300)));
        assert_eq!(m.find_next(0, &[Status::BadSector]), None);
    }

    #[test]
    fn bytes_with_status_sums_correctly() {
        let mut m = MapFile::new_non_tried(1000);
        m.mark(0, 300, Status::Finished);
        m.mark(300, 50, Status::BadSector);
        assert_eq!(m.bytes_with_status(Status::Finished), 300);
        assert_eq!(m.bytes_with_status(Status::BadSector), 50);
        assert_eq!(m.bytes_with_status(Status::NonTried), 650);
    }

    #[test]
    fn round_trips_through_ddrescue_text_format() {
        let mut m = MapFile::new_non_tried(1_000_000);
        m.mark(0, 400_000, Status::Finished);
        m.mark(400_000, 512, Status::BadSector);
        m.current_pos = 400_512;
        m.current_status = Status::NonTried;

        let text = m.to_ddrescue_string();
        let loaded = MapFile::from_ddrescue_str(&text).unwrap();

        assert_eq!(loaded.size(), m.size());
        assert_eq!(loaded.blocks(), m.blocks());
        assert_eq!(loaded.current_pos, m.current_pos);
        assert_eq!(loaded.current_status, m.current_status);
    }

    #[test]
    fn save_and_load_round_trip_on_disk() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.map");

        let mut m = MapFile::new_non_tried(2048);
        m.mark(0, 1024, Status::Finished);
        m.save(&path).unwrap();

        let loaded = MapFile::load(&path).unwrap();
        assert_eq!(loaded.blocks(), m.blocks());
    }
}
