//! Per-format header recognition and end detection for the carver. Each
//! format finds its real end from its own structure (footers, chunk/box
//! lengths, the zip central directory) rather than guessing a size.

use super::probe::Probe;
use crate::partitions::crc32;

const KIB: u64 = 1024;
const MIB: u64 = 1024 * KIB;
const GIB: u64 = 1024 * MIB;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Jpg,
    Png,
    Gif,
    Pdf,
    Zip,
    Mp4,
    Sqlite,
    SevenZ,
    Rar4,
    Rar5,
}

pub struct TypeInfo {
    pub id: &'static str,
    pub description: &'static str,
}

/// Every type id `carve` can report, in `--list-types` order.
pub const TYPES: &[TypeInfo] = &[
    TypeInfo { id: "jpg", description: "JPEG image" },
    TypeInfo { id: "png", description: "PNG image" },
    TypeInfo { id: "gif", description: "GIF image" },
    TypeInfo { id: "pdf", description: "PDF document" },
    TypeInfo { id: "zip", description: "ZIP archive" },
    TypeInfo { id: "docx", description: "Word document (.docx)" },
    TypeInfo { id: "xlsx", description: "Excel workbook (.xlsx)" },
    TypeInfo { id: "pptx", description: "PowerPoint presentation (.pptx)" },
    TypeInfo { id: "mp4", description: "MP4 video" },
    TypeInfo { id: "mov", description: "QuickTime video (.mov)" },
    TypeInfo { id: "sqlite", description: "SQLite database" },
    TypeInfo { id: "7z", description: "7-Zip archive" },
    TypeInfo { id: "rar", description: "RAR archive" },
];

impl Kind {
    pub const ALL: [Kind; 10] = [
        Kind::Jpg,
        Kind::Png,
        Kind::Gif,
        Kind::Pdf,
        Kind::Zip,
        Kind::Mp4,
        Kind::Sqlite,
        Kind::SevenZ,
        Kind::Rar4,
        Kind::Rar5,
    ];

    /// The type ids this kind can come out as.
    pub fn type_ids(self) -> &'static [&'static str] {
        match self {
            Kind::Jpg => &["jpg"],
            Kind::Png => &["png"],
            Kind::Gif => &["gif"],
            Kind::Pdf => &["pdf"],
            Kind::Zip => &["zip", "docx", "xlsx", "pptx"],
            Kind::Mp4 => &["mp4", "mov"],
            Kind::Sqlite => &["sqlite"],
            Kind::SevenZ => &["7z"],
            Kind::Rar4 | Kind::Rar5 => &["rar"],
        }
    }

    /// Anything claiming to be bigger than this is a false header match.
    pub fn max_size(self) -> u64 {
        match self {
            Kind::Jpg | Kind::Gif => 128 * MIB,
            Kind::Png => 256 * MIB,
            Kind::Pdf | Kind::Sqlite => 4 * GIB,
            Kind::Zip | Kind::SevenZ | Kind::Rar4 | Kind::Rar5 => 16 * GIB,
            Kind::Mp4 => 64 * GIB,
        }
    }

    /// How much to write when the end can't be found (bounded further by the
    /// next file header).
    pub fn unknown_end_size(self) -> u64 {
        match self {
            Kind::Jpg | Kind::Png | Kind::Gif => 16 * MIB,
            Kind::Pdf | Kind::Zip | Kind::Sqlite | Kind::SevenZ | Kind::Rar4 | Kind::Rar5 => 64 * MIB,
            Kind::Mp4 => 256 * MIB,
        }
    }

    /// How far to search for a footer before giving up.
    pub fn search_limit(self) -> u64 {
        match self {
            Kind::Pdf | Kind::Zip => GIB,
            _ => self.max_size(),
        }
    }
}

/// Recognizes a file header at the start of `b` (at least 32 bytes when
/// available).
pub fn identify(b: &[u8]) -> Option<Kind> {
    if b.len() < 16 {
        return None;
    }
    if b[0] == 0xFF && b[1] == 0xD8 && b[2] == 0xFF && b[3] >= 0xC0 && b[3] != 0xFF {
        return Some(Kind::Jpg);
    }
    if b.starts_with(b"\x89PNG\r\n\x1a\n") && &b[12..16] == b"IHDR" {
        return Some(Kind::Png);
    }
    if b.starts_with(b"GIF87a") || b.starts_with(b"GIF89a") {
        return Some(Kind::Gif);
    }
    if b.starts_with(b"%PDF-1.") || b.starts_with(b"%PDF-2.") {
        return Some(Kind::Pdf);
    }
    if b.starts_with(b"PK\x03\x04") && b[4] < 100 && b[5] == 0 {
        return Some(Kind::Zip);
    }
    if &b[4..8] == b"ftyp" {
        let size = be32(b, 0);
        if (16..=512).contains(&size) && size.is_multiple_of(4) && b[8..12].iter().all(|c| c.is_ascii_graphic() || *c == b' ') {
            // HEIF/AVIF images share the container; they aren't videos.
            if !matches!(&b[8..12], b"heic" | b"heix" | b"mif1" | b"msf1" | b"avif" | b"avis") {
                return Some(Kind::Mp4);
            }
        }
    }
    if b.starts_with(b"SQLite format 3\0") {
        return Some(Kind::Sqlite);
    }
    if b.starts_with(b"7z\xBC\xAF\x27\x1C") && b[6] == 0 {
        return Some(Kind::SevenZ);
    }
    if b.starts_with(b"Rar!\x1A\x07\x00") {
        return Some(Kind::Rar4);
    }
    if b.starts_with(b"Rar!\x1A\x07\x01\x00") {
        return Some(Kind::Rar5);
    }
    None
}

/// The file's length (if its end was found) and its final type id.
pub fn measure(kind: Kind, p: &mut Probe) -> (Option<u64>, &'static str) {
    let len = match kind {
        Kind::Jpg => jpg_end(p),
        Kind::Png => png_end(p),
        Kind::Gif => gif_end(p),
        Kind::Pdf => pdf_end(p),
        Kind::Zip => return zip_end(p),
        Kind::Mp4 => return mp4_end(p),
        Kind::Sqlite => sqlite_end(p),
        Kind::SevenZ => sevenz_end(p),
        Kind::Rar4 => rar4_end(p),
        Kind::Rar5 => rar5_end(p),
    };
    (len, kind.type_ids()[0])
}

fn jpg_end(p: &mut Probe) -> Option<u64> {
    let mut pos = 2u64;
    loop {
        let b = p.get(pos, 4)?;
        if b[0] != 0xFF {
            return None;
        }
        match b[1] {
            0xFF => pos += 1, // fill byte
            0xD9 => return Some(pos + 2),
            0x01 | 0xD0..=0xD7 => pos += 2,
            0xDA => {
                // Entropy-coded data can't contain FF D9: every data FF is
                // followed by 00 or a restart marker. Later scans of a
                // progressive JPEG sit before the EOI, so the first FF D9
                // after the first SOS is the end. EXIF thumbnails (with
                // their own EOI) live in APP1 and were skipped by length.
                let after = pos + 2 + be16(&b, 2) as u64;
                return p.find(after, b"\xFF\xD9").map(|e| e + 2);
            }
            _ => {
                let len = be16(&b, 2) as u64;
                if len < 2 {
                    return None;
                }
                pos += 2 + len;
            }
        }
    }
}

fn png_end(p: &mut Probe) -> Option<u64> {
    let mut pos = 8u64;
    loop {
        let b = p.get(pos, 8)?;
        let len = be32(&b, 0) as u64;
        if len > 0x7FFF_FFFF || !b[4..8].iter().all(|c| c.is_ascii_alphabetic()) {
            return None;
        }
        pos += 12 + len;
        if &b[4..8] == b"IEND" {
            return Some(pos);
        }
    }
}

fn gif_end(p: &mut Probe) -> Option<u64> {
    let lsd = p.get(6, 7)?;
    let mut pos = 13u64;
    if lsd[4] & 0x80 != 0 {
        pos += 3 << ((lsd[4] & 7) + 1);
    }
    loop {
        match p.get(pos, 1)?[0] {
            0x3B => return Some(pos + 1),
            0x21 => {
                pos = skip_gif_subblocks(p, pos + 2)?;
            }
            0x2C => {
                let d = p.get(pos, 10)?;
                pos += 10;
                if d[9] & 0x80 != 0 {
                    pos += 3 << ((d[9] & 7) + 1);
                }
                pos = skip_gif_subblocks(p, pos + 1)?; // +1: LZW minimum code size
            }
            _ => return None,
        }
    }
}

fn skip_gif_subblocks(p: &mut Probe, mut pos: u64) -> Option<u64> {
    loop {
        let n = p.get(pos, 1)?[0] as u64;
        pos += 1;
        if n == 0 {
            return Some(pos);
        }
        pos += n;
    }
}

/// The last `%%EOF` that isn't followed by an incremental update.
fn pdf_end(p: &mut Probe) -> Option<u64> {
    let mut from = 0u64;
    loop {
        let at = p.find(from, b"%%EOF")?;
        let mut end = at + 5;
        let after = p.get_upto(end, 64);
        let mut i = 0;
        // The EOL after %%EOF belongs to the file.
        if after.get(i) == Some(&b'\r') {
            i += 1;
        }
        if after.get(i) == Some(&b'\n') {
            i += 1;
        }
        end += i as u64;
        if !looks_like_pdf_update(&after[i..]) {
            return Some(end);
        }
        from = end;
    }
}

fn looks_like_pdf_update(b: &[u8]) -> bool {
    let s: Vec<u8> = b.iter().copied().skip_while(|c| c.is_ascii_whitespace()).collect();
    if s.starts_with(b"xref") || s.starts_with(b"%") {
        return true;
    }
    // "12 0 obj"
    let mut it = s.split(|c| *c == b' ');
    let (a, g, o) = (it.next(), it.next(), it.next());
    matches!((a, g, o), (Some(a), Some(g), Some(o))
        if !a.is_empty() && a.iter().all(u8::is_ascii_digit)
        && !g.is_empty() && g.iter().all(u8::is_ascii_digit)
        && o.starts_with(b"obj"))
}

/// Finds the end-of-central-directory record that belongs to *this* zip
/// (its central directory must end exactly where the EOCD starts — that
/// rejects the EOCD of a zip stored inside this one), and reads the
/// central directory to tell docx/xlsx/pptx from a plain zip.
fn zip_end(p: &mut Probe) -> (Option<u64>, &'static str) {
    let mut from = 4u64;
    while let Some(at) = p.find(from, b"PK\x05\x06") {
        from = at + 4;
        let Some(e) = p.get(at, 22) else { break };
        let (mut cd_size, mut cd_off) = (le32(&e, 12) as u64, le32(&e, 16) as u64);
        let mut cd_end = at;
        if cd_off == 0xFFFF_FFFF || cd_size == 0xFFFF_FFFF {
            let Some(loc) = p.get(at.saturating_sub(20), 20).filter(|l| l.starts_with(b"PK\x06\x07")) else {
                continue;
            };
            let z64_at = le64(&loc, 8);
            let Some(z) = p.get(z64_at, 56).filter(|z| z.starts_with(b"PK\x06\x06")) else {
                continue;
            };
            cd_size = le64(&z, 40);
            cd_off = le64(&z, 48);
            cd_end = z64_at;
        }
        if cd_off.checked_add(cd_size) != Some(cd_end) {
            continue;
        }
        let end = at + 22 + le16(&e, 20) as u64;
        return (Some(end), office_kind(p, cd_off, cd_size));
    }
    (None, "zip")
}

fn office_kind(p: &mut Probe, cd_off: u64, cd_size: u64) -> &'static str {
    let Some(cd) = p.get(cd_off, cd_size.min(8 * MIB) as usize) else {
        return "zip";
    };
    let mut i = 0usize;
    while i + 46 <= cd.len() && cd[i..].starts_with(b"PK\x01\x02") {
        let (n, x, c) = (le16(&cd, i + 28) as usize, le16(&cd, i + 30) as usize, le16(&cd, i + 32) as usize);
        let name = &cd[(i + 46).min(cd.len())..(i + 46 + n).min(cd.len())];
        match name {
            b"word/document.xml" => return "docx",
            b"xl/workbook.xml" => return "xlsx",
            b"ppt/presentation.xml" => return "pptx",
            _ => {}
        }
        i += 46 + n + x + c;
    }
    "zip"
}

const MP4_TOP_LEVEL: &[&[u8; 4]] = &[
    b"moov", b"mdat", b"free", b"skip", b"wide", b"uuid", b"pdin", b"meta", b"moof", b"mfra", b"styp", b"sidx",
    b"ssix", b"prft", b"emsg", b"pnot", b"junk",
];

/// Walks top-level boxes. Complete only once both `moov` (the index) and
/// `mdat` (the media) have been seen — without `moov` a video won't play.
fn mp4_end(p: &mut Probe) -> (Option<u64>, &'static str) {
    let kind = match p.get(8, 4).as_deref() {
        Some(b"qt  ") => "mov",
        _ => "mp4",
    };
    let first = p.get(0, 4).map(|b| be32(&b, 0) as u64).unwrap_or(0);
    let mut pos = first;
    let (mut moov, mut mdat) = (false, false);
    while let Some(b) = p.get(pos, 16) {
        let typ: [u8; 4] = b[4..8].try_into().unwrap();
        if !MP4_TOP_LEVEL.contains(&&typ) {
            break;
        }
        let mut size = be32(&b, 0) as u64;
        if size == 1 {
            size = be64(&b, 8);
        }
        if size < 8 {
            // 0 = "runs to the end of the file", which is exactly what we
            // don't know.
            return (None, kind);
        }
        moov |= &typ == b"moov";
        mdat |= &typ == b"mdat";
        pos = match pos.checked_add(size) {
            Some(p) => p,
            None => return (None, kind),
        };
        if pos > p.limit() {
            return (None, kind);
        }
    }
    ((moov && mdat).then_some(pos), kind)
}

fn sqlite_end(p: &mut Probe) -> Option<u64> {
    let h = p.get(0, 100)?;
    let page_size = match be16(&h, 16) {
        1 => 65536,
        n if n >= 512 && n.is_power_of_two() => n as u64,
        _ => return None,
    };
    let pages = be32(&h, 28) as u64;
    // The in-header page count is only trustworthy when written by a
    // SQLite that also stamped "version-valid-for" with the change counter.
    (pages > 0 && be32(&h, 92) == be32(&h, 24)).then_some(page_size * pages)
}

fn sevenz_end(p: &mut Probe) -> Option<u64> {
    let h = p.get(0, 32)?;
    if crc32(&h[12..32]) != le32(&h, 8) {
        return None;
    }
    32u64.checked_add(le64(&h, 12))?.checked_add(le64(&h, 20))
}

fn rar4_end(p: &mut Probe) -> Option<u64> {
    let mut pos = 0u64;
    let mut files = 0;
    loop {
        let h = p.get(pos, 7)?;
        let (typ, flags, size) = (h[2], le16(&h, 3), le16(&h, 5) as u64);
        if !(0x72..=0x7B).contains(&typ) || size < 7 {
            // Old RAR versions don't always write an end block; a clean
            // run of blocks followed by something else ends the archive.
            return (files > 0).then_some(pos);
        }
        let mut add = 0u64;
        if flags & 0x8000 != 0 {
            add = le32(&p.get(pos + 7, 4)?, 0) as u64;
            if typ == 0x74 && flags & 0x100 != 0 {
                add += (le32(&p.get(pos + 32, 4)?, 0) as u64) << 32;
            }
        }
        if typ == 0x74 {
            files += 1;
        }
        pos += size + add;
        if typ == 0x7B {
            return Some(pos);
        }
    }
}

fn rar5_end(p: &mut Probe) -> Option<u64> {
    let mut pos = 8u64;
    loop {
        let b = p.get_upto(pos, 4 + 10 * 5);
        if b.len() < 6 {
            return None;
        }
        let mut i = 4usize;
        let header_size = vint(&b, &mut i)?;
        let header_start = i;
        let typ = vint(&b, &mut i)?;
        let flags = vint(&b, &mut i)?;
        if flags & 1 != 0 {
            vint(&b, &mut i)?;
        }
        let data = if flags & 2 != 0 { vint(&b, &mut i)? } else { 0 };
        if !(1..=5).contains(&typ) || header_size == 0 || header_size > 2 * MIB {
            return None;
        }
        pos = pos.checked_add(header_start as u64 + header_size)?.checked_add(data)?;
        if typ == 5 {
            return Some(pos);
        }
    }
}

fn vint(b: &[u8], i: &mut usize) -> Option<u64> {
    let mut v = 0u64;
    for shift in (0..70).step_by(7) {
        let byte = *b.get(*i)?;
        *i += 1;
        v |= ((byte & 0x7F) as u64) << shift;
        if byte & 0x80 == 0 {
            return Some(v);
        }
    }
    None
}

fn be16(b: &[u8], o: usize) -> u16 {
    u16::from_be_bytes([b[o], b[o + 1]])
}
fn be32(b: &[u8], o: usize) -> u32 {
    u32::from_be_bytes(b[o..o + 4].try_into().unwrap())
}
fn be64(b: &[u8], o: usize) -> u64 {
    u64::from_be_bytes(b[o..o + 8].try_into().unwrap())
}
fn le16(b: &[u8], o: usize) -> u16 {
    u16::from_le_bytes([b[o], b[o + 1]])
}
fn le32(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes(b[o..o + 4].try_into().unwrap())
}
fn le64(b: &[u8], o: usize) -> u64 {
    u64::from_le_bytes(b[o..o + 8].try_into().unwrap())
}
