//! Windowed, bounded reads relative to a candidate file's start, so format
//! parsers can walk structures without caring about sectors or the device.

use crate::reader::SafeReader;

const WINDOW: usize = 1024 * 1024;

pub struct Probe<'r, 'a> {
    reader: &'r mut SafeReader<'a>,
    start: u64,
    limit: u64,
    win_off: u64,
    win: Vec<u8>,
}

impl<'r, 'a> Probe<'r, 'a> {
    /// Reads are allowed in `[start, start + limit)`, clipped to the device.
    pub fn new(reader: &'r mut SafeReader<'a>, start: u64, limit: u64) -> Self {
        let limit = limit.min(reader.size().saturating_sub(start));
        Self { reader, start, limit, win_off: 0, win: Vec::new() }
    }

    pub fn limit(&self) -> u64 {
        self.limit
    }

    fn load(&mut self, rel: u64, len: usize) {
        let len = len.max(WINDOW).min((self.limit - rel) as usize);
        self.win.resize(len, 0);
        self.reader.read(self.start + rel, &mut self.win);
        self.win_off = rel;
    }

    fn in_window(&self, rel: u64, len: usize) -> bool {
        rel >= self.win_off && rel + len as u64 <= self.win_off + self.win.len() as u64
    }

    /// Exactly `len` bytes at `rel`, or `None` past the limit. Unreadable
    /// bytes come back as zeros (the carver checks for those separately).
    pub fn get(&mut self, rel: u64, len: usize) -> Option<Vec<u8>> {
        if rel.checked_add(len as u64)? > self.limit {
            return None;
        }
        if !self.in_window(rel, len) {
            self.load(rel, len);
        }
        let o = (rel - self.win_off) as usize;
        Some(self.win[o..o + len].to_vec())
    }

    /// Up to `len` bytes at `rel` (fewer near the limit).
    pub fn get_upto(&mut self, rel: u64, len: usize) -> Vec<u8> {
        let len = (len as u64).min(self.limit.saturating_sub(rel)) as usize;
        self.get(rel, len).unwrap_or_default()
    }

    /// Offset of the first `pat` at or after `from`.
    pub fn find(&mut self, from: u64, pat: &[u8]) -> Option<u64> {
        let mut pos = from;
        while pos + pat.len() as u64 <= self.limit {
            if !self.in_window(pos, pat.len()) {
                self.load(pos, pat.len());
            }
            let o = (pos - self.win_off) as usize;
            if let Some(i) = self.win[o..].windows(pat.len()).position(|w| w == pat) {
                return Some(pos + i as u64);
            }
            let win_end = self.win_off + self.win.len() as u64;
            if win_end >= self.limit {
                return None;
            }
            pos = win_end - (pat.len() as u64 - 1);
        }
        None
    }
}
