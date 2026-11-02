//! Directory entry parsing for FAT (8.3 names plus VFAT long names) and
//! exFAT (file entry sets). Both keep deleted entries in place with one
//! byte changed, so deleted names come back too.

use super::{u16_at, u32_at, u64_at, Loc};
use crate::timefmt::dos_to_filetime;

const DELETED: u8 = 0xE5;
const ATTR_DIR: u8 = 0x10;
const ATTR_VOLUME: u8 = 0x08;
const ATTR_LFN: u8 = 0x0F;

#[derive(Debug, Clone)]
pub(super) struct RawEntry {
    pub name: String,
    pub is_dir: bool,
    pub deleted: bool,
    pub loc: Loc,
    pub created: u64,
    pub modified: u64,
}

pub(super) fn lfn_checksum(short: &[u8]) -> u8 {
    short.iter().fold(0u8, |s, &c| s.rotate_right(1).wrapping_add(c))
}

/// Plausible as an 8.3 name: printable, none of the characters FAT forbids.
fn valid_short(e: &[u8], deleted: bool) -> bool {
    let first_ok = deleted || e[0] == 0x05 || e[0] > 0x20;
    first_ok && e[1..11].iter().all(|&c| c >= 0x20 && !b"\"*+,/:;<=>?[\\]|".contains(&c)) && e[0..8] != [b' '; 8]
}

fn short_name(e: &[u8], deleted: bool) -> String {
    let case = e[12];
    let part = |b: &[u8], lower: bool| -> String {
        let s: String = b.iter().map(|&c| char::from(c)).collect::<String>().trim_end().to_string();
        if lower { s.to_lowercase() } else { s }
    };
    let mut base = e[0..8].to_vec();
    base[0] = match base[0] {
        0x05 => DELETED,
        _ if deleted => b'_', // the real first character is gone
        c => c,
    };
    let (base, ext) = (part(&base, case & 0x08 != 0), part(&e[8..11], case & 0x10 != 0));
    if ext.is_empty() { base } else { format!("{base}.{ext}") }
}

/// The long name from the LFN entries just before an 8.3 entry (in disk
/// order, so last part first), if they belong to it.
fn long_name(parts: &[&[u8]], short: &[u8], deleted: bool) -> Option<String> {
    let sum = parts.first()?[13];
    if parts.iter().any(|p| p[13] != sum) {
        return None;
    }
    // A deleted entry's first byte was overwritten, so neither the 8.3
    // checksum nor the sequence numbers can be checked; agreeing checksums
    // among the parts have to do.
    if deleted {
        if parts.iter().any(|p| p[0] != DELETED) {
            return None;
        }
    } else {
        let ords_ok = parts.iter().enumerate().all(|(i, p)| {
            let want = (parts.len() - i) as u8 | if i == 0 { 0x40 } else { 0 };
            p[0] == want
        });
        if !ords_ok || lfn_checksum(&short[0..11]) != sum {
            return None;
        }
    }
    let mut units = Vec::with_capacity(parts.len() * 13);
    for p in parts.iter().rev() {
        for r in [1..11, 14..26, 28..32] {
            units.extend(p[r].chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])));
        }
    }
    let end = units.iter().position(|&u| u == 0).unwrap_or(units.len());
    let name = String::from_utf16_lossy(&units[..end]);
    (!name.is_empty()).then_some(name)
}

/// Entries of a FAT directory. With `strict` (a deleted directory, whose
/// clusters may run on into other data), stops at the first entry that
/// doesn't look like one instead of skipping it.
pub(super) fn parse_fat(buf: &[u8], fat32: bool, strict: bool) -> Vec<RawEntry> {
    if strict && !buf.starts_with(b".          ") {
        return Vec::new(); // its first cluster no longer holds this directory
    }
    let mut out = Vec::new();
    let mut lfn: Vec<&[u8]> = Vec::new();
    for e in buf.chunks_exact(32) {
        if e[0] == 0 {
            break;
        }
        let attr = e[11];
        if attr & 0x3F == ATTR_LFN {
            lfn.push(e);
            continue;
        }
        let parts = std::mem::take(&mut lfn);
        let deleted = e[0] == DELETED;
        if attr & ATTR_VOLUME != 0 || e[0] == b'.' {
            continue;
        }
        if attr & 0xC0 != 0 || !valid_short(e, deleted) {
            if strict {
                break;
            }
            continue;
        }
        let is_dir = attr & ATTR_DIR != 0;
        let hi = if fat32 { (u16_at(e, 20) as u32) << 16 } else { 0 };
        let size = if is_dir { 0 } else { u32_at(e, 28) as u64 };
        out.push(RawEntry {
            name: long_name(&parts, e, deleted).unwrap_or_else(|| short_name(e, deleted)),
            is_dir,
            deleted,
            loc: Loc { first: hi | u16_at(e, 26) as u32, size, valid: size, contiguous: false, deleted },
            created: dos_to_filetime(u16_at(e, 16), u16_at(e, 14), e[13], 0),
            modified: dos_to_filetime(u16_at(e, 24), u16_at(e, 22), 0, 0),
        });
    }
    out
}

/// (first cluster, length) of the allocation bitmap, from the root.
pub(super) fn exfat_bitmap(buf: &[u8]) -> Option<(u32, u64)> {
    buf.chunks_exact(32).take_while(|e| e[0] != 0).find(|e| e[0] == 0x81).map(|e| (u32_at(e, 20), u64_at(e, 24)))
}

pub(super) fn set_checksum(set: &[u8]) -> u16 {
    set.iter().enumerate().filter(|(i, _)| !matches!(i, 2 | 3)).fold(0u16, |s, (i, &b)| {
        // Deleting clears bit 7 of each type byte; check as if it were set.
        let b = if i % 32 == 0 { b | 0x80 } else { b };
        s.rotate_right(1).wrapping_add(b as u16)
    })
}

/// exFAT timestamp, 10 ms increment and UTC offset byte → FILETIME.
fn exfat_time(ts: u32, ms10: u8, off: u8) -> u64 {
    let minutes = if off & 0x80 != 0 { ((off << 1) as i8 >> 1) as i32 * 15 } else { 0 };
    dos_to_filetime((ts >> 16) as u16, ts as u16, ms10, minutes)
}

/// File entry sets (file + stream extension + names) in an exFAT
/// directory, live and deleted. Sets whose checksum fails are skipped.
pub(super) fn parse_exfat(buf: &[u8]) -> Vec<RawEntry> {
    let mut out = Vec::new();
    let mut i = 0;
    while i + 32 <= buf.len() {
        let t = buf[i];
        if t == 0 {
            break;
        }
        let in_use = t & 0x80;
        let count = buf[i + 1] as usize;
        let end = i + 32 * (count + 1);
        if t & 0x7F != 0x05 || !(2..=18).contains(&count) || end > buf.len() {
            i += 32;
            continue;
        }
        let set = &buf[i..end];
        let types_ok = set[32] == 0x40 | in_use && set[64..].chunks_exact(32).all(|n| n[0] == 0x41 | in_use);
        if !types_ok || set_checksum(set) != u16_at(set, 2) {
            i += 32;
            continue;
        }
        let stream = &set[32..64];
        let name_len = stream[3] as usize;
        let units: Vec<u16> = set[64..].chunks_exact(32).flat_map(|n| n[2..32].chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]]))).take(name_len).collect();
        let size = u64_at(stream, 24);
        let deleted = in_use == 0;
        out.push(RawEntry {
            name: String::from_utf16_lossy(&units),
            is_dir: u16_at(set, 4) & ATTR_DIR as u16 != 0,
            deleted,
            loc: Loc {
                first: u32_at(stream, 20),
                size,
                valid: u64_at(stream, 8).min(size),
                contiguous: stream[1] & 2 != 0,
                deleted,
            },
            created: exfat_time(u32_at(set, 8), set[20], set[22]),
            modified: exfat_time(u32_at(set, 12), set[21], set[23]),
        });
        i = end;
    }
    out
}
