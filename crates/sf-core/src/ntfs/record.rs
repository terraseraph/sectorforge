//! MFT record parsing: update-sequence fixups, the record header, and the
//! attributes `files`/`extract` need. Everything is bounds-checked; a record
//! that doesn't parse cleanly is rejected rather than half-trusted.

use super::runlist::{decode_runlist, Run};

pub const ATTR_STANDARD_INFORMATION: u32 = 0x10;
pub const ATTR_ATTRIBUTE_LIST: u32 = 0x20;
pub const ATTR_FILE_NAME: u32 = 0x30;
pub const ATTR_DATA: u32 = 0x80;
const ATTR_END: u32 = 0xFFFF_FFFF;

pub const FLAG_COMPRESSED: u16 = 0x0001;
pub const FLAG_ENCRYPTED: u16 = 0x4000;

/// Applies the update-sequence fixups in place. `false` means the record is
/// torn (a sector was written without the others, or replaced by zeros
/// because it couldn't be read) and must not be trusted.
pub fn apply_fixups(rec: &mut [u8]) -> bool {
    if rec.len() < 48 || &rec[0..4] != b"FILE" {
        return false;
    }
    let usa_off = u16_at(rec, 4) as usize;
    let usa_count = u16_at(rec, 6) as usize;
    let sectors = rec.len() / 512;
    if usa_count == 0 || usa_count - 1 != sectors || usa_off + usa_count * 2 > rec.len() || usa_off < 0x28 {
        return false;
    }
    let usn = [rec[usa_off], rec[usa_off + 1]];
    for i in 0..sectors {
        let end = (i + 1) * 512;
        // A sector that was unreadable comes back as zeros; with a zero USN
        // it would pass the check below, so reject it explicitly.
        if rec[end - 512..end].iter().all(|&b| b == 0) {
            return false;
        }
        if rec[end - 2..end] != usn {
            return false;
        }
        let fix = usa_off + 2 + i * 2;
        rec[end - 2] = rec[fix];
        rec[end - 1] = rec[fix + 1];
    }
    true
}

#[derive(Debug, Clone)]
pub struct Record {
    pub seq: u16,
    pub in_use: bool,
    pub is_dir: bool,
    /// Non-zero for an extension record: the base record's file reference.
    pub base_ref: u64,
    pub attrs: Vec<Attr>,
}

#[derive(Debug, Clone)]
pub struct Attr {
    pub typ: u32,
    pub id: u16,
    pub flags: u16,
    pub named: bool,
    pub body: AttrBody,
}

#[derive(Debug, Clone)]
pub enum AttrBody {
    Resident(Vec<u8>),
    NonResident(NonResident),
}

#[derive(Debug, Clone)]
pub struct NonResident {
    pub start_vcn: u64,
    pub last_vcn: u64,
    pub runs: Vec<Run>,
    pub data_size: u64,
    pub init_size: u64,
}

/// Parses a fixed-up record. `None` if any structure is out of bounds.
pub fn parse_record(rec: &[u8]) -> Option<Record> {
    let flags = u16_at(rec, 0x16);
    let used = (u32_at(rec, 0x18) as usize).min(rec.len());
    let mut off = u16_at(rec, 0x14) as usize;
    let mut attrs = Vec::new();
    while off + 8 <= used {
        let typ = u32_at(rec, off);
        if typ == ATTR_END {
            break;
        }
        let len = u32_at(rec, off + 4) as usize;
        if len < 16 || !len.is_multiple_of(8) || off + len > used {
            return None;
        }
        let a = &rec[off..off + len];
        attrs.push(parse_attr(a)?);
        off += len;
    }
    Some(Record {
        seq: u16_at(rec, 0x10),
        in_use: flags & 1 != 0,
        is_dir: flags & 2 != 0,
        base_ref: u64_at(rec, 0x20),
        attrs,
    })
}

fn parse_attr(a: &[u8]) -> Option<Attr> {
    let non_resident = a[8] != 0;
    let name_len = a[9];
    let flags = u16_at(a, 0x0C);
    let id = u16_at(a, 0x0E);
    let body = if !non_resident {
        if a.len() < 0x18 {
            return None;
        }
        let vlen = u32_at(a, 0x10) as usize;
        let voff = u16_at(a, 0x14) as usize;
        AttrBody::Resident(a.get(voff..voff.checked_add(vlen)?)?.to_vec())
    } else {
        if a.len() < 0x40 {
            return None;
        }
        let start_vcn = u64_at(a, 0x10);
        let rl_off = u16_at(a, 0x20) as usize;
        let runs = decode_runlist(a.get(rl_off..)?, start_vcn)?;
        AttrBody::NonResident(NonResident {
            start_vcn,
            last_vcn: u64_at(a, 0x18),
            runs,
            data_size: u64_at(a, 0x30),
            init_size: u64_at(a, 0x38),
        })
    };
    Some(Attr {
        typ: u32_at(a, 0),
        id,
        flags,
        named: name_len != 0,
        body,
    })
}

#[derive(Debug, Clone)]
pub struct FileName {
    pub parent_ref: u64,
    pub namespace: u8,
    pub name: String,
    pub created: u64,
    pub modified: u64,
}

pub fn parse_file_name(v: &[u8]) -> Option<FileName> {
    if v.len() < 0x42 {
        return None;
    }
    let n = v[0x40] as usize;
    let units: Vec<u16> = v
        .get(0x42..0x42 + n * 2)?
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .collect();
    Some(FileName {
        parent_ref: u64_at(v, 0),
        namespace: v[0x41],
        name: String::from_utf16_lossy(&units),
        created: u64_at(v, 0x08),
        modified: u64_at(v, 0x10),
    })
}

/// Win32 (or Win32+DOS) names are the real ones; POSIX next; the 8.3 DOS
/// alias only if nothing else exists.
pub fn namespace_rank(ns: u8) -> u8 {
    match ns {
        1 | 3 => 0,
        0 => 1,
        _ => 2,
    }
}

/// (created, modified) FILETIMEs from `$STANDARD_INFORMATION`.
pub fn parse_std_info(v: &[u8]) -> Option<(u64, u64)> {
    (v.len() >= 16).then(|| (u64_at(v, 0), u64_at(v, 8)))
}

#[derive(Debug, Clone)]
pub struct AttrListEntry {
    pub typ: u32,
    pub record_ref: u64,
    pub id: u16,
}

pub fn parse_attr_list(v: &[u8]) -> Vec<AttrListEntry> {
    let mut out = Vec::new();
    let mut off = 0usize;
    while off + 0x1A <= v.len() {
        let len = u16_at(v, off + 4) as usize;
        if len < 0x1A || off + len > v.len() {
            break;
        }
        out.push(AttrListEntry {
            typ: u32_at(v, off),
            record_ref: u64_at(v, off + 0x10),
            id: u16_at(v, off + 0x18),
        });
        off += len;
    }
    out
}

pub fn record_number(file_ref: u64) -> u64 {
    file_ref & 0x0000_FFFF_FFFF_FFFF
}

pub fn ref_seq(file_ref: u64) -> u16 {
    (file_ref >> 48) as u16
}

pub(crate) fn u16_at(b: &[u8], o: usize) -> u16 {
    b.get(o..o + 2).map_or(0, |s| u16::from_le_bytes([s[0], s[1]]))
}
pub(crate) fn u32_at(b: &[u8], o: usize) -> u32 {
    b.get(o..o + 4).map_or(0, |s| u32::from_le_bytes(s.try_into().unwrap()))
}
pub(crate) fn u64_at(b: &[u8], o: usize) -> u64 {
    b.get(o..o + 8).map_or(0, |s| u64::from_le_bytes(s.try_into().unwrap()))
}
