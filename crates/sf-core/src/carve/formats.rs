//! Per-format header recognition and end detection for the carver. Each
//! format finds its real end from its own structure (footers, chunk/box
//! lengths, the zip central directory) rather than guessing a size. Formats
//! with no recoverable end (compressed streams, opaque headers) are still
//! recognized; the carver then writes a bounded guess marked incomplete.

use super::probe::Probe;
use crate::partitions::crc32;

const KIB: u64 = 1024;
const MIB: u64 = 1024 * KIB;
const GIB: u64 = 1024 * MIB;

/// Bytes `identify` looks at. Big enough for a tar header, five TS packets
/// or a PE header behind a typical DOS stub.
pub const HEADER_LEN: usize = 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Jpg,
    Png,
    Gif,
    Bmp,
    Tiff,
    Ico,
    Psd,
    Jp2,
    Xcf,
    Pdf,
    Ole,
    Rtf,
    Ps,
    Djvu,
    Chm,
    Zip,
    SevenZ,
    Rar4,
    Rar5,
    Gz,
    Bz2,
    Xz,
    Zstd,
    Tar,
    Cab,
    Ar,
    Mp4,
    Riff,
    Mkv,
    Flv,
    Asf,
    Ts,
    Mpg,
    Mp3,
    Flac,
    Ogg,
    Aiff,
    Midi,
    Amr,
    Sqlite,
    Exe,
    Elf,
    Pst,
    Kdbx,
    Lnk,
    Regf,
    Evtx,
    Font,
    Woff,
    Blend,
    Dwg,
}

pub struct TypeInfo {
    pub id: &'static str,
    pub description: &'static str,
}

macro_rules! types {
    ($($id:literal => $d:literal),* $(,)?) => { &[$(TypeInfo { id: $id, description: $d }),*] };
}

/// Every type id `carve` can report, in `--list-types` order.
pub const TYPES: &[TypeInfo] = types![
    "jpg" => "JPEG image",
    "png" => "PNG image",
    "gif" => "GIF image",
    "bmp" => "BMP image",
    "tif" => "TIFF image",
    "webp" => "WebP image",
    "heic" => "HEIC/HEIF image",
    "avif" => "AVIF image",
    "ico" => "Windows icon",
    "cur" => "Windows cursor",
    "psd" => "Photoshop image",
    "psb" => "Photoshop large document (.psb)",
    "jp2" => "JPEG 2000 image",
    "xcf" => "GIMP image",
    "cr2" => "Canon RAW photo (.cr2)",
    "cr3" => "Canon RAW photo (.cr3)",
    "nef" => "Nikon RAW photo",
    "arw" => "Sony RAW photo",
    "dng" => "Adobe DNG RAW photo",
    "orf" => "Olympus RAW photo",
    "rw2" => "Panasonic RAW photo",
    "pdf" => "PDF document",
    "doc" => "Word 97-2003 document (.doc)",
    "xls" => "Excel 97-2003 workbook (.xls)",
    "ppt" => "PowerPoint 97-2003 presentation (.ppt)",
    "msg" => "Outlook message (.msg)",
    "ole" => "Other OLE compound file (msi, vsd, pub, ...)",
    "docx" => "Word document (.docx)",
    "xlsx" => "Excel workbook (.xlsx)",
    "pptx" => "PowerPoint presentation (.pptx)",
    "vsdx" => "Visio drawing (.vsdx)",
    "xps" => "XPS document",
    "odt" => "OpenDocument text",
    "ods" => "OpenDocument spreadsheet",
    "odp" => "OpenDocument presentation",
    "odg" => "OpenDocument drawing",
    "epub" => "EPUB e-book",
    "rtf" => "Rich Text document",
    "ps" => "PostScript document",
    "eps" => "Encapsulated PostScript",
    "djvu" => "DjVu document",
    "chm" => "Compiled HTML help",
    "zip" => "ZIP archive",
    "jar" => "Java archive",
    "apk" => "Android package",
    "7z" => "7-Zip archive",
    "rar" => "RAR archive",
    "gz" => "gzip archive",
    "bz2" => "bzip2 archive",
    "xz" => "xz archive",
    "zst" => "Zstandard archive",
    "tar" => "tar archive",
    "cab" => "Windows cabinet",
    "deb" => "Debian package",
    "a" => "ar archive / static library",
    "mp4" => "MP4 video",
    "mov" => "QuickTime video (.mov)",
    "m4v" => "iTunes video (.m4v)",
    "3gp" => "3GP phone video",
    "avi" => "AVI video",
    "mkv" => "Matroska video",
    "webm" => "WebM video",
    "flv" => "Flash video",
    "wmv" => "Windows Media video",
    "ts" => "MPEG transport stream",
    "m2ts" => "Blu-ray/AVCHD video (.m2ts)",
    "mpg" => "MPEG program stream / DVD video",
    "mp3" => "MP3 audio",
    "m4a" => "AAC audio (.m4a)",
    "wav" => "WAV audio",
    "flac" => "FLAC audio",
    "ogg" => "Ogg audio/video",
    "opus" => "Opus audio",
    "aiff" => "AIFF audio",
    "wma" => "Windows Media audio",
    "mid" => "MIDI music",
    "amr" => "AMR voice recording",
    "sqlite" => "SQLite database",
    "exe" => "Windows program",
    "dll" => "Windows library (.dll)",
    "elf" => "Linux/Unix program (ELF)",
    "pst" => "Outlook data file (.pst)",
    "ost" => "Outlook offline data file (.ost)",
    "kdbx" => "KeePass database",
    "lnk" => "Windows shortcut",
    "dat" => "Windows registry hive",
    "evtx" => "Windows event log",
    "ttf" => "TrueType font",
    "otf" => "OpenType font",
    "woff" => "Web font (.woff)",
    "woff2" => "Web font (.woff2)",
    "blend" => "Blender scene",
    "dwg" => "AutoCAD drawing",
    "3mf" => "3D print model (.3mf)",
];

impl Kind {
    pub const ALL: [Kind; 51] = [
        Kind::Jpg,
        Kind::Png,
        Kind::Gif,
        Kind::Bmp,
        Kind::Tiff,
        Kind::Ico,
        Kind::Psd,
        Kind::Jp2,
        Kind::Xcf,
        Kind::Pdf,
        Kind::Ole,
        Kind::Rtf,
        Kind::Ps,
        Kind::Djvu,
        Kind::Chm,
        Kind::Zip,
        Kind::SevenZ,
        Kind::Rar4,
        Kind::Rar5,
        Kind::Gz,
        Kind::Bz2,
        Kind::Xz,
        Kind::Zstd,
        Kind::Tar,
        Kind::Cab,
        Kind::Ar,
        Kind::Mp4,
        Kind::Riff,
        Kind::Mkv,
        Kind::Flv,
        Kind::Asf,
        Kind::Ts,
        Kind::Mpg,
        Kind::Mp3,
        Kind::Flac,
        Kind::Ogg,
        Kind::Aiff,
        Kind::Midi,
        Kind::Amr,
        Kind::Sqlite,
        Kind::Exe,
        Kind::Elf,
        Kind::Pst,
        Kind::Kdbx,
        Kind::Lnk,
        Kind::Regf,
        Kind::Evtx,
        Kind::Font,
        Kind::Woff,
        Kind::Blend,
        Kind::Dwg,
    ];

    /// The type ids this kind can come out as.
    pub fn type_ids(self) -> &'static [&'static str] {
        match self {
            Kind::Jpg => &["jpg"],
            Kind::Png => &["png"],
            Kind::Gif => &["gif"],
            Kind::Bmp => &["bmp"],
            Kind::Tiff => &["tif", "cr2", "nef", "arw", "dng", "orf", "rw2"],
            Kind::Ico => &["ico", "cur"],
            Kind::Psd => &["psd", "psb"],
            Kind::Jp2 => &["jp2"],
            Kind::Xcf => &["xcf"],
            Kind::Pdf => &["pdf"],
            Kind::Ole => &["doc", "xls", "ppt", "msg", "ole"],
            Kind::Rtf => &["rtf"],
            Kind::Ps => &["ps", "eps"],
            Kind::Djvu => &["djvu"],
            Kind::Chm => &["chm"],
            Kind::Zip => {
                &["zip", "docx", "xlsx", "pptx", "vsdx", "xps", "odt", "ods", "odp", "odg", "epub", "jar", "apk", "3mf"]
            }
            Kind::SevenZ => &["7z"],
            Kind::Rar4 | Kind::Rar5 => &["rar"],
            Kind::Gz => &["gz"],
            Kind::Bz2 => &["bz2"],
            Kind::Xz => &["xz"],
            Kind::Zstd => &["zst"],
            Kind::Tar => &["tar"],
            Kind::Cab => &["cab"],
            Kind::Ar => &["a", "deb"],
            Kind::Mp4 => &["mp4", "mov", "m4v", "3gp", "m4a", "heic", "avif", "cr3"],
            Kind::Riff => &["avi", "wav", "webp"],
            Kind::Mkv => &["mkv", "webm"],
            Kind::Flv => &["flv"],
            Kind::Asf => &["wma", "wmv"],
            Kind::Ts => &["ts", "m2ts"],
            Kind::Mpg => &["mpg"],
            Kind::Mp3 => &["mp3"],
            Kind::Flac => &["flac"],
            Kind::Ogg => &["ogg", "opus"],
            Kind::Aiff => &["aiff"],
            Kind::Midi => &["mid"],
            Kind::Amr => &["amr"],
            Kind::Sqlite => &["sqlite"],
            Kind::Exe => &["exe", "dll"],
            Kind::Elf => &["elf"],
            Kind::Pst => &["pst", "ost"],
            Kind::Kdbx => &["kdbx"],
            Kind::Lnk => &["lnk"],
            Kind::Regf => &["dat"],
            Kind::Evtx => &["evtx"],
            Kind::Font => &["ttf", "otf"],
            Kind::Woff => &["woff", "woff2"],
            Kind::Blend => &["blend"],
            Kind::Dwg => &["dwg"],
        }
    }

    /// Anything claiming to be bigger than this is a false header match.
    pub fn max_size(self) -> u64 {
        match self {
            Kind::Lnk => MIB,
            Kind::Ico | Kind::Midi | Kind::Font | Kind::Woff | Kind::Amr => 64 * MIB,
            Kind::Jpg | Kind::Gif => 128 * MIB,
            Kind::Png | Kind::Kdbx => 256 * MIB,
            Kind::Bmp | Kind::Jp2 | Kind::Xcf | Kind::Djvu | Kind::Chm | Kind::Rtf | Kind::Ps => GIB,
            Kind::Mp3 | Kind::Flac | Kind::Exe | Kind::Elf | Kind::Dwg | Kind::Evtx => GIB,
            Kind::Pdf | Kind::Sqlite | Kind::Ole | Kind::Tiff | Kind::Aiff | Kind::Cab | Kind::Regf | Kind::Blend => {
                4 * GIB
            }
            Kind::Zip | Kind::SevenZ | Kind::Rar4 | Kind::Rar5 | Kind::Psd => 16 * GIB,
            Kind::Gz | Kind::Bz2 | Kind::Xz | Kind::Zstd | Kind::Tar | Kind::Ar => 16 * GIB,
            Kind::Mp4 | Kind::Riff | Kind::Mkv | Kind::Flv | Kind::Asf | Kind::Ts | Kind::Mpg | Kind::Ogg => 64 * GIB,
            Kind::Pst => 64 * GIB,
        }
    }

    /// How much to write when the end can't be found (bounded further by the
    /// next file header).
    pub fn unknown_end_size(self) -> u64 {
        match self {
            Kind::Lnk => MIB,
            Kind::Jpg | Kind::Png | Kind::Gif | Kind::Bmp | Kind::Ico | Kind::Midi | Kind::Font | Kind::Woff => {
                16 * MIB
            }
            Kind::Amr | Kind::Kdbx | Kind::Mp3 => 16 * MIB,
            Kind::Mp4 | Kind::Riff | Kind::Mkv | Kind::Flv | Kind::Asf | Kind::Ts | Kind::Mpg | Kind::Ogg => 256 * MIB,
            _ => 64 * MIB,
        }
    }

    /// How far to search for a footer before giving up.
    pub fn search_limit(self) -> u64 {
        match self {
            Kind::Pdf | Kind::Zip | Kind::Ps | Kind::Xz => GIB,
            _ => self.max_size(),
        }
    }
}

const OLE_MAGIC: &[u8] = b"\xD0\xCF\x11\xE0\xA1\xB1\x1A\xE1";
const ASF_HEADER: &[u8] = b"\x30\x26\xB2\x75\x8E\x66\xCF\x11\xA6\xD9\x00\xAA\x00\x62\xCE\x6C";
const ASF_FILE_PROPS: &[u8] = b"\xA1\xDC\xAB\x8C\x47\xA9\xCF\x11\x8E\xE4\x00\xC0\x0C\x20\x53\x65";
const ASF_STREAM_PROPS: &[u8] = b"\x91\x07\xDC\xB7\xB7\xA9\xCF\x11\x8E\xE6\x00\xC0\x0C\x20\x53\x65";
const ASF_VIDEO: &[u8] = b"\xC0\xEF\x19\xBC\x4D\x5B\xCF\x11\xA8\xFD\x00\x80\x5F\x5C\x44\x2B";
const LNK_CLSID: &[u8] = b"\x01\x14\x02\x00\x00\x00\x00\x00\xC0\x00\x00\x00\x00\x00\x00\x46";

/// Recognizes a file header at the start of `b` (up to [`HEADER_LEN`]
/// bytes). Every signature here also ends the previous unknown-length file,
/// so each one validates more than a short magic.
pub fn identify(b: &[u8]) -> Option<Kind> {
    if b.len() < 16 {
        return None;
    }
    // Zero-padded when short (near the end of the device) so checks can
    // index freely; copied only then, since this runs on every sector.
    let pad;
    let b = if b.len() >= HEADER_LEN {
        b
    } else {
        pad = {
            let mut p = [0u8; HEADER_LEN];
            p[..b.len()].copy_from_slice(b);
            p
        };
        &pad[..]
    };

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
        if (16..=512).contains(&size)
            && size.is_multiple_of(4)
            && b[8..12].iter().all(|c| c.is_ascii_graphic() || *c == b' ')
        {
            return Some(Kind::Mp4);
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
    if b.starts_with(OLE_MAGIC) && le16(b, 0x1C) == 0xFFFE && matches!((le16(b, 0x1A), le16(b, 0x1E)), (3, 9) | (4, 12))
    {
        return Some(Kind::Ole);
    }
    if b.starts_with(b"{\\rtf1") {
        return Some(Kind::Rtf);
    }
    if b.starts_with(b"%!PS-Adobe-") || (b.starts_with(b"\xC5\xD0\xD3\xC6") && le32(b, 4) == 30) {
        return Some(Kind::Ps);
    }
    if b.starts_with(b"AT&TFORM") && matches!(&b[12..16], b"DJVU" | b"DJVM" | b"DJVI" | b"THUM") {
        return Some(Kind::Djvu);
    }
    if b.starts_with(b"ITSF") && le32(b, 4) == 3 && matches!(le32(b, 8), 0x58 | 0x60) {
        return Some(Kind::Chm);
    }
    if bmp_ok(b) {
        return Some(Kind::Bmp);
    }
    if tiff_ifd0(b).is_some() {
        return Some(Kind::Tiff);
    }
    if ico_ok(b) {
        return Some(Kind::Ico);
    }
    if psd_ok(b) {
        return Some(Kind::Psd);
    }
    if b.starts_with(b"\0\0\0\x0CjP  \r\n\x87\n") {
        return Some(Kind::Jp2);
    }
    if b.starts_with(b"gimp xcf ") && (&b[9..14] == b"file\0" || (b[9] == b'v' && b[13] == 0)) {
        return Some(Kind::Xcf);
    }
    if b.starts_with(b"\x1F\x8B\x08") && b[3] & 0xE0 == 0 && matches!(b[8], 0 | 2 | 4) && (b[9] <= 13 || b[9] == 255) {
        return Some(Kind::Gz);
    }
    if b.starts_with(b"BZh")
        && (b'1'..=b'9').contains(&b[3])
        && matches!(&b[4..10], b"\x31\x41\x59\x26\x53\x59" | b"\x17\x72\x45\x38\x50\x90")
    {
        return Some(Kind::Bz2);
    }
    if b.starts_with(b"\xFD7zXZ\0") && b[6] == 0 && matches!(b[7], 0 | 1 | 4 | 10) && crc32(&b[6..8]) == le32(b, 8) {
        return Some(Kind::Xz);
    }
    if le32(b, 0) == 0xFD2F_B528 && zstd_first_block_ok(b) {
        return Some(Kind::Zstd);
    }
    if &b[257..262] == b"ustar" && tar_checksum_ok(&b[..512]) {
        return Some(Kind::Tar);
    }
    if b.starts_with(b"MSCF") && le32(b, 4) == 0 && le32(b, 12) == 0 && le32(b, 20) == 0 && b[24] == 3 && b[25] == 1 {
        return Some(Kind::Cab);
    }
    if b.starts_with(b"!<arch>\n") && &b[66..68] == b"`\n" {
        return Some(Kind::Ar);
    }
    if b.starts_with(b"RIFF") && matches!(&b[8..12], b"WAVE" | b"AVI " | b"WEBP") {
        return Some(Kind::Riff);
    }
    if b.starts_with(b"\x1A\x45\xDF\xA3")
        && (b[4..64].windows(8).any(|w| w == b"matroska") || b[4..64].windows(4).any(|w| w == b"webm"))
    {
        return Some(Kind::Mkv);
    }
    if b.starts_with(b"FLV\x01") && b[4] & 0xFA == 0 && be32(b, 5) == 9 && be32(b, 9) == 0 {
        return Some(Kind::Flv);
    }
    if b.starts_with(ASF_HEADER) {
        return Some(Kind::Asf);
    }
    if ts_layout(b).is_some() {
        return Some(Kind::Ts);
    }
    if mpg_pack_len(b).is_some() {
        return Some(Kind::Mpg);
    }
    if b.starts_with(b"ID3")
        && (2..=4).contains(&b[3])
        && b[4] != 0xFF
        && b[5] & 0x0F == 0
        && b[6..10].iter().all(|&c| c < 0x80)
    {
        return Some(Kind::Mp3);
    }
    if b.starts_with(b"fLaC") && b[4] & 0x7F == 0 && be24(b, 5) == 34 {
        return Some(Kind::Flac);
    }
    if b.starts_with(b"OggS") && b[4] == 0 && b[5] == 2 && le64(b, 6) == 0 && le32(b, 18) == 0 {
        return Some(Kind::Ogg);
    }
    if b.starts_with(b"FORM") && matches!(&b[8..12], b"AIFF" | b"AIFC") {
        return Some(Kind::Aiff);
    }
    if b.starts_with(b"MThd") && be32(b, 4) == 6 && be16(b, 8) <= 2 && be16(b, 10) >= 1 {
        return Some(Kind::Midi);
    }
    if b.starts_with(b"#!AMR\n") || b.starts_with(b"#!AMR-WB\n") {
        return Some(Kind::Amr);
    }
    if pe_offset(b).is_some() {
        return Some(Kind::Exe);
    }
    if b.starts_with(b"\x7FELF") && matches!(b[4], 1 | 2) && matches!(b[5], 1 | 2) && b[6] == 1 {
        let le = b[5] == 1;
        if (1..=4).contains(&uint(b, 16, 2, le)) && uint(b, 20, 4, le) == 1 {
            return Some(Kind::Elf);
        }
    }
    if b.starts_with(b"!BDN") && matches!(&b[8..10], b"SM" | b"SO") && matches!(le16(b, 10), 14 | 15 | 23 | 36) {
        return Some(Kind::Pst);
    }
    if b.starts_with(b"\x03\xD9\xA2\x9A\x67\xFB\x4B\xB5") {
        return Some(Kind::Kdbx);
    }
    if le32(b, 0) == 0x4C && &b[4..20] == LNK_CLSID {
        return Some(Kind::Lnk);
    }
    if regf_ok(b) {
        return Some(Kind::Regf);
    }
    if b.starts_with(b"ElfFile\0") && crc32(&b[..120]) == le32(b, 124) {
        return Some(Kind::Evtx);
    }
    if font_ok(b) {
        return Some(Kind::Font);
    }
    if (b.starts_with(b"wOFF") || b.starts_with(b"wOF2")) && be16(b, 12) > 0 && be16(b, 14) == 0 {
        return Some(Kind::Woff);
    }
    if b.starts_with(b"BLENDER")
        && matches!(b[7], b'_' | b'-')
        && matches!(b[8], b'v' | b'V')
        && b[9..12].iter().all(u8::is_ascii_digit)
    {
        return Some(Kind::Blend);
    }
    if b.starts_with(b"AC10") && b[4..6].iter().all(u8::is_ascii_digit) && b[6..11].iter().all(|&c| c == 0) {
        return Some(Kind::Dwg);
    }
    None
}

/// The file's length (if its end was found) and its final type id.
pub fn measure(kind: Kind, p: &mut Probe) -> (Option<u64>, &'static str) {
    let len = match kind {
        Kind::Jpg => jpg_end(p),
        Kind::Png => png_end(p),
        Kind::Gif => gif_end(p),
        Kind::Bmp => bmp_end(p),
        Kind::Tiff => return tiff_end(p),
        Kind::Ico => return ico_end(p),
        Kind::Psd => return psd_end(p),
        Kind::Jp2 => jp2_end(p),
        Kind::Pdf => pdf_end(p),
        Kind::Ole => return ole_end(p),
        Kind::Rtf => rtf_end(p),
        Kind::Ps => return ps_end(p),
        Kind::Djvu => p.get(8, 4).map(|h| 12 + be32(&h, 0) as u64),
        Kind::Chm => chm_end(p),
        Kind::Zip => return zip_end(p),
        Kind::SevenZ => sevenz_end(p),
        Kind::Rar4 => rar4_end(p),
        Kind::Rar5 => rar5_end(p),
        Kind::Xz => xz_end(p),
        Kind::Zstd => zstd_end(p),
        Kind::Tar => tar_end(p),
        Kind::Cab => p.get(8, 4).map(|h| le32(&h, 0) as u64),
        Kind::Ar => return ar_end(p),
        Kind::Mp4 => return mp4_end(p),
        Kind::Riff => return riff_end(p),
        Kind::Mkv => return mkv_end(p),
        Kind::Flv => flv_end(p),
        Kind::Asf => return asf_end(p),
        Kind::Ts => return ts_end(p),
        Kind::Mpg => mpg_end(p),
        Kind::Mp3 => mp3_end(p),
        Kind::Ogg => return ogg_end(p),
        Kind::Aiff => p.get(4, 4).map(|h| (8 + be32(&h, 0) as u64 + 1) & !1),
        Kind::Midi => midi_end(p),
        Kind::Sqlite => sqlite_end(p),
        Kind::Exe => return pe_end(p),
        Kind::Elf => elf_end(p),
        Kind::Pst => return pst_end(p),
        Kind::Lnk => lnk_end(p),
        Kind::Regf => regf_end(p),
        Kind::Evtx => evtx_end(p),
        Kind::Font => return font_end(p),
        Kind::Woff => return woff_end(p),
        Kind::Blend => blend_end(p),
        // Compressed streams with no length or bit-aligned trailers, and
        // formats whose length hides in encrypted/opaque headers.
        Kind::Gz | Kind::Bz2 | Kind::Flac | Kind::Amr | Kind::Xcf | Kind::Kdbx | Kind::Dwg => None,
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

fn bmp_ok(b: &[u8]) -> bool {
    let dib = le32(b, 14);
    if !b.starts_with(b"BM") || le32(b, 6) != 0 || !matches!(dib, 12 | 40 | 52 | 56 | 64 | 108 | 124) {
        return false;
    }
    let (planes, bpp) = if dib == 12 { (le16(b, 22), le16(b, 24)) } else { (le16(b, 26), le16(b, 28)) };
    le32(b, 10) >= 14 + dib && planes == 1 && matches!(bpp, 1 | 4 | 8 | 16 | 24 | 32)
}

fn bmp_end(p: &mut Probe) -> Option<u64> {
    let h = p.get(0, 34)?;
    let (size, off) = (le32(&h, 2) as u64, le32(&h, 10) as u64);
    if size > off {
        return Some(size);
    }
    // Uncompressed files may leave the size 0; the dimensions give it.
    if le32(&h, 14) < 40 || le32(&h, 30) != 0 {
        return None;
    }
    let (w, rows, bpp) =
        ((le32(&h, 18) as i32).unsigned_abs() as u64, (le32(&h, 22) as i32).unsigned_abs() as u64, le16(&h, 28) as u64);
    Some(off + (w * bpp).div_ceil(32) * 4 * rows)
}

/// Byte order and first IFD offset of a TIFF-structured file (TIFF and the
/// camera raws built on it, which swap the 42 for their own marker).
fn tiff_ifd0(b: &[u8]) -> Option<(bool, u64)> {
    let le = match &b[..4] {
        b"II*\0" | b"IIRO" | b"IIRS" | b"IIU\0" => true,
        b"MM\0*" | b"MMOR" => false,
        _ => return None,
    };
    let ifd = uint(b, 4, 4, le);
    (ifd >= 8 && ifd.is_multiple_of(2) && ifd < 0x8000_0000).then_some((le, ifd))
}

const TIFF_TYPE_SIZE: [u64; 14] = [0, 1, 1, 2, 4, 8, 1, 1, 2, 4, 8, 4, 8, 4];

/// Walks every IFD (chain, SubIFDs, EXIF, GPS, interop) and takes the
/// furthest byte any entry or image strip/tile points at. Raws are typed
/// by their header marker, the DNG version tag or the camera make.
fn tiff_end(p: &mut Probe) -> (Option<u64>, &'static str) {
    let Some((h, (le, ifd0))) = p.get(0, 16).and_then(|h| tiff_ifd0(&h).map(|t| (h, t))) else {
        return (None, "tif");
    };
    let mut kind = match (&h[2..4], &h[8..10]) {
        (b"*\0", b"CR") => "cr2",
        (b"RO" | b"RS" | b"OR", _) => "orf",
        (b"U\0", _) => "rw2",
        _ => "tif",
    };
    let end = tiff_walk(p, le, ifd0, &mut kind);
    // ponytail: RW2 raw data runs from RawDataOffset to EOF with no stored
    // length, so it stays header-only; parse the maker data to fix that.
    (end.filter(|_| kind != "rw2"), kind)
}

fn tiff_walk(p: &mut Probe, le: bool, ifd0: u64, kind: &mut &'static str) -> Option<u64> {
    let (mut todo, mut seen, mut end) = (vec![ifd0], Vec::new(), 8u64);
    while let Some(ifd) = todo.pop() {
        if seen.contains(&ifd) || seen.len() >= 64 {
            continue;
        }
        seen.push(ifd);
        let n = uint(&p.get(ifd, 2)?, 0, 2, le);
        if n == 0 || n > 1000 {
            return None;
        }
        let e = p.get(ifd + 2, 12 * n as usize + 4)?;
        end = end.max(ifd + 6 + 12 * n);
        // (offsets, byte counts) for strips, tiles and the JPEG thumbnail.
        let mut parts: [(Vec<u64>, Vec<u64>); 3] = Default::default();
        for f in e[..12 * n as usize].chunks(12) {
            let (tag, typ, count) = (uint(f, 0, 2, le), uint(f, 2, 2, le), uint(f, 4, 4, le));
            // Unknown types are skipped, as the spec tells readers to.
            let Some(&unit) = TIFF_TYPE_SIZE.get(typ as usize).filter(|&&s| s > 0) else { continue };
            let size = unit * count;
            if size > 4 {
                end = end.max(uint(f, 8, 4, le) + size);
            }
            match tag {
                273 => parts[0].0 = tiff_values(p, le, typ, count, f)?,
                279 => parts[0].1 = tiff_values(p, le, typ, count, f)?,
                324 => parts[1].0 = tiff_values(p, le, typ, count, f)?,
                325 => parts[1].1 = tiff_values(p, le, typ, count, f)?,
                513 => parts[2].0 = tiff_values(p, le, typ, count, f)?,
                514 => parts[2].1 = tiff_values(p, le, typ, count, f)?,
                330 | 34665 | 34853 | 40965 => todo.extend(tiff_values(p, le, typ, count, f)?),
                50706 => *kind = "dng",
                271 if *kind == "tif" => {
                    let make = if size <= 4 { f[8..12].to_vec() } else { p.get_upto(uint(f, 8, 4, le), 8) };
                    if make.starts_with(b"NIKON") {
                        *kind = "nef";
                    } else if make.starts_with(b"SONY") {
                        *kind = "arw";
                    }
                }
                _ => {}
            }
        }
        for (offs, lens) in &parts {
            for (o, l) in offs.iter().zip(lens) {
                end = end.max(o + l);
            }
        }
        let next = uint(&e, 12 * n as usize, 4, le);
        if next != 0 {
            todo.push(next);
        }
    }
    Some(end)
}

/// The SHORT/LONG values of an IFD entry, inline or out of line.
fn tiff_values(p: &mut Probe, le: bool, typ: u64, count: u64, f: &[u8]) -> Option<Vec<u64>> {
    let w = match typ {
        3 => 2,
        4 | 13 => 4,
        _ => return Some(Vec::new()),
    };
    if count > 1 << 20 {
        return None;
    }
    let len = (count * w) as usize;
    let data = if len <= 4 { f[8..8 + len].to_vec() } else { p.get(uint(f, 8, 4, le), len)? };
    Some(data.chunks(w as usize).map(|c| uint(c, 0, w as usize, le)).collect())
}

/// Reserved zero, a known type, and the first image right after the
/// directory (where every writer puts it).
fn ico_ok(b: &[u8]) -> bool {
    let n = le16(b, 4) as u32;
    b[..2] == [0, 0]
        && matches!(le16(b, 2), 1 | 2)
        && (1..=255).contains(&n)
        && b[9] == 0
        && le32(b, 14) > 0
        && le32(b, 18) == 6 + 16 * n
}

fn ico_end(p: &mut Probe) -> (Option<u64>, &'static str) {
    let kind = if p.get(2, 1).is_some_and(|t| t[0] == 2) { "cur" } else { "ico" };
    let end = (|| {
        let n = le16(&p.get(4, 2)?, 0) as u64;
        let mut end = 6 + 16 * n;
        for e in p.get(6, 16 * n as usize)?.chunks(16) {
            let (size, off) = (le32(e, 8) as u64, le32(e, 12) as u64);
            if e[3] != 0 || size == 0 || off < 6 + 16 * n {
                return None;
            }
            end = end.max(off + size);
        }
        Some(end)
    })();
    (end, kind)
}

fn psd_ok(b: &[u8]) -> bool {
    b.starts_with(b"8BPS")
        && matches!(be16(b, 4), 1 | 2)
        && b[6..12] == [0; 6]
        && (1..=56).contains(&be16(b, 12))
        && (1..=300_000).contains(&be32(b, 14))
        && (1..=300_000).contains(&be32(b, 18))
        && matches!(be16(b, 22), 1 | 8 | 16 | 32)
        && be16(b, 24) <= 9
}

/// Skips the three length-prefixed sections, then sizes the merged image
/// data from its dimensions (raw) or its per-row byte counts (RLE).
fn psd_end(p: &mut Probe) -> (Option<u64>, &'static str) {
    let big = p.get(4, 2).is_some_and(|v| v[1] == 2);
    let end = (|| {
        let h = p.get(0, 26)?;
        let lines = be16(&h, 12) as u64 * be32(&h, 14) as u64;
        let row = (be32(&h, 18) as u64 * be16(&h, 22) as u64).div_ceil(8);
        let mut pos = 26u64;
        for wide in [false, false, big] {
            let len = if wide { be64(&p.get(pos, 8)?, 0).checked_add(8)? } else { be32(&p.get(pos, 4)?, 0) as u64 + 4 };
            pos = pos.checked_add(len)?;
        }
        let comp = be16(&p.get(pos, 2)?, 0);
        pos += 2;
        match comp {
            0 => Some(pos + lines * row),
            1 => {
                let w = if big { 4 } else { 2 };
                let (mut sum, mut i) = (0u64, 0u64);
                while i < lines {
                    let n = (lines - i).min(64 * KIB);
                    sum += p
                        .get(pos + i * w, (n * w) as usize)?
                        .chunks(w as usize)
                        .map(|c| uint(c, 0, w as usize, false))
                        .sum::<u64>();
                    i += n;
                }
                Some(pos + lines * w + sum)
            }
            _ => None, // zip-compressed image data has no stored length
        }
    })();
    (end, if big { "psb" } else { "psd" })
}

const JP2_TOP_LEVEL: &[&[u8; 4]] =
    &[b"jP  ", b"ftyp", b"rreq", b"jp2h", b"jp2i", b"jp2c", b"xml ", b"uuid", b"uinf", b"ipr ", b"res "];

/// Complete once the codestream box has been walked past.
fn jp2_end(p: &mut Probe) -> Option<u64> {
    let (end, seen) = walk_boxes(p, 0, JP2_TOP_LEVEL);
    end.filter(|_| seen.contains(b"jp2c"))
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

const OLE_FREE: u32 = 0xFFFF_FFFF;

/// Size from the highest sector the FAT marks as used; type from the
/// streams directly under the root storage (embedded objects live deeper,
/// so a .doc holding a spreadsheet is still a .doc).
fn ole_end(p: &mut Probe) -> (Option<u64>, &'static str) {
    let Some((fat, ss, dir)) = ole_fat(p) else {
        return (None, "ole");
    };
    let Some(last) = fat.iter().rposition(|&e| e != OLE_FREE) else {
        return (None, "ole");
    };
    (Some((last as u64 + 2) * ss), ole_kind(p, &fat, ss, dir))
}

/// The whole FAT, the sector size and the first directory sector.
fn ole_fat(p: &mut Probe) -> Option<(Vec<u32>, u64, u32)> {
    let h = p.get(0, 512)?;
    let ss = 1u64 << le16(&h, 0x1E);
    let per = (ss / 4) as usize;
    let n_fat = le32(&h, 0x2C) as usize;
    if n_fat == 0 || (n_fat * per) as u64 * ss > Kind::Ole.max_size() {
        return None;
    }
    // The first 109 FAT sector numbers are in the header, the rest in a
    // chain of DIFAT sectors.
    let mut secs: Vec<u32> = (0..109).map(|i| le32(&h, 0x4C + 4 * i)).collect();
    let mut difat = le32(&h, 0x44);
    while secs.len() < n_fat {
        let d = p.get((difat as u64 + 1) * ss, ss as usize)?;
        secs.extend((0..per - 1).map(|i| le32(&d, 4 * i)));
        difat = le32(&d, ss as usize - 4);
    }
    let mut fat = Vec::with_capacity(n_fat * per);
    for &s in &secs[..n_fat] {
        fat.extend(p.get((s as u64 + 1) * ss, ss as usize)?.chunks(4).map(|c| le32(c, 0)));
    }
    Some((fat, ss, le32(&h, 0x30)))
}

fn ole_kind(p: &mut Probe, fat: &[u32], ss: u64, mut sec: u32) -> &'static str {
    let mut dir = Vec::new();
    // ponytail: reads at most 64 directory sectors; the root's children
    // come first in every real file.
    for _ in 0..64 {
        let Some(d) = fat.get(sec as usize).and_then(|_| p.get((sec as u64 + 1) * ss, ss as usize)) else { break };
        dir.extend(d);
        sec = fat[sec as usize];
    }
    let (entries, _) = dir.as_chunks::<128>();
    // The root's children form a tree linked by left/right sibling ids.
    let mut todo = vec![entries.first().map_or(OLE_FREE, |e| le32(e, 76))];
    let (mut kind, mut visits) = ("ole", 0);
    while let Some(i) = todo.pop() {
        let Some(e) = entries.get(i as usize) else { continue };
        visits += 1;
        if visits > entries.len() {
            break;
        }
        todo.extend([le32(e, 68), le32(e, 72)]);
        let units: Vec<u16> = e[..(le16(e, 64) as usize).clamp(2, 64) - 2].chunks(2).map(|c| le16(c, 0)).collect();
        match String::from_utf16_lossy(&units).as_str() {
            n if n.starts_with("__substg1.0_") => return "msg",
            "WordDocument" => kind = "doc",
            "Workbook" | "Book" => kind = "xls",
            "PowerPoint Document" => kind = "ppt",
            _ => {}
        }
    }
    kind
}

/// The brace that closes the opening `{\rtf1`.
// ponytail: `\binN` raw bytes aren't skipped, so a brace inside embedded
// binary data can end the file early; handle \bin if that shows up.
fn rtf_end(p: &mut Probe) -> Option<u64> {
    let (mut depth, mut pos, mut escaped) = (0u32, 0u64, false);
    loop {
        let chunk = p.get_upto(pos, 64 * KIB as usize);
        if chunk.is_empty() {
            return None;
        }
        for (i, &c) in chunk.iter().enumerate() {
            if escaped {
                escaped = false;
                continue;
            }
            match c {
                b'\\' => escaped = true,
                b'{' => depth += 1,
                b'}' => {
                    depth = depth.checked_sub(1)?;
                    if depth == 0 {
                        return Some(pos + i as u64 + 1);
                    }
                }
                0 => return None, // RTF is 7-bit text; a NUL means we ran past it
                _ => {}
            }
        }
        pos += chunk.len() as u64;
    }
}

/// DOS EPS binaries list their sections; text PostScript ends at the
/// `%%EOF` that isn't inside an embedded `%%BeginDocument` (an EPS figure).
fn ps_end(p: &mut Probe) -> (Option<u64>, &'static str) {
    let h = p.get_upto(0, 64);
    if h.starts_with(b"\xC5\xD0\xD3\xC6") && h.len() >= 28 {
        let end = [4, 12, 20].iter().map(|&o| le32(&h, o) as u64 + le32(&h, o + 4) as u64).max();
        return (end, "eps");
    }
    let first_line = h.split(|&c| c == b'\n' || c == b'\r').next().unwrap_or_default();
    let kind = if first_line.windows(5).any(|w| w == b"EPSF-") { "eps" } else { "ps" };
    let (mut from, mut depth) = (0u64, 0u32);
    while let Some(at) = p.find(from, b"%%") {
        let t = p.get_upto(at + 2, 13);
        if t.starts_with(b"BeginDocument") {
            depth += 1;
        } else if t.starts_with(b"EndDocument") {
            depth = depth.saturating_sub(1);
        } else if t.starts_with(b"EOF") && depth == 0 {
            return (Some(skip_eol(p, at + 5)), kind);
        }
        from = at + 2;
    }
    (None, kind)
}

/// `end` moved past one CR, LF or CRLF.
fn skip_eol(p: &mut Probe, end: u64) -> u64 {
    let a = p.get_upto(end, 2);
    let cr = (a.first() == Some(&b'\r')) as usize;
    end + cr as u64 + (a.get(cr) == Some(&b'\n')) as u64
}

fn chm_end(p: &mut Probe) -> Option<u64> {
    let sec0 = le64(&p.get(0x38, 8)?, 0);
    let s = p.get(sec0, 16)?;
    (le32(&s, 0) == 0x01FE).then_some(le64(&s, 8))
}

/// Finds the end-of-central-directory record that belongs to *this* zip
/// (its central directory must end exactly where the EOCD starts — that
/// rejects the EOCD of a zip stored inside this one), and reads the
/// central directory to tell the zip-based formats from a plain zip.
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
    let mut kind = "zip";
    let mut i = 0usize;
    while i + 46 <= cd.len() && cd[i..].starts_with(b"PK\x01\x02") {
        let (n, x, c) = (le16(&cd, i + 28) as usize, le16(&cd, i + 30) as usize, le16(&cd, i + 32) as usize);
        let name = &cd[(i + 46).min(cd.len())..(i + 46 + n).min(cd.len())];
        match name {
            b"word/document.xml" => return "docx",
            b"xl/workbook.xml" => return "xlsx",
            b"ppt/presentation.xml" => return "pptx",
            b"visio/document.xml" => return "vsdx",
            b"3D/3dmodel.model" => return "3mf",
            b"AndroidManifest.xml" => return "apk",
            _ if name.ends_with(b".fdseq") => return "xps",
            b"mimetype" => {
                if let Some(k) = zip_mimetype(p, le32(&cd, i + 42) as u64) {
                    return k;
                }
            }
            // Signed APKs and other Java-ish zips have one too: a fallback.
            b"META-INF/MANIFEST.MF" => kind = "jar",
            _ => {}
        }
        i += 46 + n + x + c;
    }
    kind
}

/// OpenDocument and EPUB name their type in a stored `mimetype` entry.
fn zip_mimetype(p: &mut Probe, local: u64) -> Option<&'static str> {
    let h = p.get(local, 30)?;
    let data = p.get_upto(local + 30 + le16(&h, 26) as u64 + le16(&h, 28) as u64, 64);
    ZIP_MIMETYPES.iter().find(|(m, _)| data.starts_with(m)).map(|&(_, k)| k)
}

const ZIP_MIMETYPES: &[(&[u8], &str)] = &[
    (b"application/epub+zip", "epub"),
    (b"application/vnd.oasis.opendocument.text", "odt"),
    (b"application/vnd.oasis.opendocument.spreadsheet", "ods"),
    (b"application/vnd.oasis.opendocument.presentation", "odp"),
    (b"application/vnd.oasis.opendocument.graphics", "odg"),
];

const MP4_TOP_LEVEL: &[&[u8; 4]] = &[
    b"moov", b"mdat", b"free", b"skip", b"wide", b"uuid", b"pdin", b"meta", b"moof", b"mfra", b"styp", b"sidx",
    b"ssix", b"prft", b"emsg", b"pnot", b"junk",
];

/// Walks top-level boxes. A video is complete only once both `moov` (the
/// index) and `mdat` (the media) have been seen — without `moov` it won't
/// play; a HEIF/AVIF image needs `meta` instead of `moov`.
fn mp4_end(p: &mut Probe) -> (Option<u64>, &'static str) {
    let kind = mp4_kind(&p.get_upto(0, 512));
    let first = p.get(0, 4).map(|b| be32(&b, 0) as u64).unwrap_or(0);
    let (end, seen) = walk_boxes(p, first, MP4_TOP_LEVEL);
    let index: &[u8; 4] = if matches!(kind, "heic" | "avif") { b"meta" } else { b"moov" };
    (end.filter(|_| seen.contains(index) && seen.contains(b"mdat")), kind)
}

fn mp4_kind(ftyp: &[u8]) -> &'static str {
    let compat = &ftyp[16.min(ftyp.len())..(be32(ftyp, 0) as usize).min(ftyp.len())];
    match &ftyp[8..12] {
        b"qt  " => "mov",
        b"M4A " | b"M4B " | b"M4P " => "m4a",
        b"M4V " | b"M4VH" | b"M4VP" => "m4v",
        b"crx " => "cr3",
        b"avif" | b"avis" => "avif",
        b"heic" | b"heix" | b"heim" | b"heis" | b"hevc" | b"hevx" | b"mif1" | b"msf1" => {
            if compat.chunks(4).any(|c| c == b"avif") {
                "avif"
            } else {
                "heic"
            }
        }
        m if m.starts_with(b"3gp") || m.starts_with(b"3g2") => "3gp",
        _ => "mp4",
    }
}

/// Walks ISO-style boxes (32-bit size + type) from `pos` while the type is
/// one of `top`. The end is `None` when a box runs to "end of file" (size
/// 0), which is exactly what we don't know.
fn walk_boxes(p: &mut Probe, mut pos: u64, top: &[&[u8; 4]]) -> (Option<u64>, Vec<[u8; 4]>) {
    let mut seen = Vec::new();
    while let Some(b) = p.get_upto(pos, 16).get(..8).map(<[u8]>::to_vec) {
        let typ: [u8; 4] = b[4..8].try_into().unwrap();
        if !top.contains(&&typ) {
            break;
        }
        let mut size = be32(&b, 0) as u64;
        if size == 1 {
            size = p.get(pos + 8, 8).map_or(0, |l| be64(&l, 0));
        }
        if size < 8 {
            return (None, seen);
        }
        seen.push(typ);
        match pos.checked_add(size) {
            Some(n) if n <= p.limit() => pos = n,
            _ => return (None, seen),
        }
    }
    (Some(pos), seen)
}

/// WAV/AVI/WebP from the RIFF size; AVIs over 1 GiB continue in further
/// `RIFF AVIX` chunks right after the first.
fn riff_end(p: &mut Probe) -> (Option<u64>, &'static str) {
    let Some(h) = p.get(0, 12) else { return (None, "avi") };
    let kind = match &h[8..12] {
        b"WAVE" => "wav",
        b"WEBP" => "webp",
        _ => "avi",
    };
    let mut end = (8 + le32(&h, 4) as u64 + 1) & !1;
    while let Some(n) = p.get(end, 12).filter(|n| kind == "avi" && n.starts_with(b"RIFF") && &n[8..12] == b"AVIX") {
        end += (8 + le32(&n, 4) as u64 + 1) & !1;
    }
    ((le32(&h, 4) >= 4).then_some(end), kind)
}

/// The EBML header, then the Segment's declared size. Live-recorded files
/// leave the size "unknown" (all ones); those fall back.
fn mkv_end(p: &mut Probe) -> (Option<u64>, &'static str) {
    let h = p.get_upto(0, 64);
    let kind = if h.windows(4).any(|w| w == b"webm") { "webm" } else { "mkv" };
    let end = (|| {
        let mut i = 4;
        let hsize = ebml_size(&h, &mut i)?;
        let seg = i as u64 + hsize;
        let s = p.get(seg, 12)?;
        if !s.starts_with(b"\x18\x53\x80\x67") {
            return None;
        }
        let mut j = 4;
        let size = ebml_size(&s, &mut j)?;
        (seg + j as u64).checked_add(size)
    })();
    (end, kind)
}

/// An EBML variable-length size; `None` for the reserved "unknown" value.
fn ebml_size(b: &[u8], i: &mut usize) -> Option<u64> {
    let first = *b.get(*i)?;
    let len = first.leading_zeros() as usize + 1;
    if len > 8 {
        return None;
    }
    let mask = (0xFFu16 >> len) as u8;
    let mut v = (first & mask) as u64;
    let mut all_ones = v == mask as u64;
    for k in 1..len {
        let c = *b.get(*i + k)?;
        all_ones &= c == 0xFF;
        v = v << 8 | c as u64;
    }
    *i += len;
    (!all_ones).then_some(v)
}

/// Every tag is followed by the previous tag's size, so a clean walk ends
/// exactly where the tags stop agreeing with themselves.
fn flv_end(p: &mut Probe) -> Option<u64> {
    let (mut pos, mut tags) = (13u64, 0);
    while let Some(t) = p.get(pos, 11) {
        let size = be24(&t, 1) as u64;
        if !matches!(t[0] & 0x1F, 8 | 9 | 18) || t[8..11] != [0, 0, 0] {
            break;
        }
        if p.get(pos + 11 + size, 4).is_none_or(|prev| be32(&prev, 0) as u64 != 11 + size) {
            break;
        }
        pos += 15 + size;
        tags += 1;
    }
    (tags > 0).then_some(pos)
}

/// The file size from the File Properties header object; WMV when any
/// stream is video.
fn asf_end(p: &mut Probe) -> (Option<u64>, &'static str) {
    let Some(h) = p.get(0, 30) else { return (None, "wma") };
    let (hsize, count) = (le64(&h, 16), le32(&h, 24));
    let objs = p.get_upto(30, hsize.saturating_sub(30).min(MIB) as usize);
    let (mut i, mut size, mut video) = (0usize, None, false);
    for _ in 0..count {
        if i + 40 > objs.len() {
            break;
        }
        let (guid, osize) = (&objs[i..i + 16], le64(&objs, i + 16) as usize);
        if guid == ASF_FILE_PROPS && i + 48 <= objs.len() {
            size = Some(le64(&objs, i + 40));
        }
        video |= guid == ASF_STREAM_PROPS && &objs[i + 24..i + 40] == ASF_VIDEO;
        if osize < 24 {
            break;
        }
        i = i.saturating_add(osize);
    }
    // Broadcast files leave the size 0 because they're still being written.
    (size.filter(|&s| s >= hsize), if video { "wmv" } else { "wma" })
}

/// Packet size and sync byte position: plain TS (188) or M2TS/AVCHD (192,
/// with a 4-byte timestamp first). Several consecutive packets must agree.
fn ts_layout(b: &[u8]) -> Option<(u64, u64)> {
    if (0..5).all(|i| b[i * 188] == 0x47) {
        Some((188, 0))
    } else if (0..4).all(|i| b[4 + i * 192] == 0x47) {
        Some((192, 4))
    } else {
        None
    }
}

/// Packets run until the sync byte stops repeating.
fn ts_end(p: &mut Probe) -> (Option<u64>, &'static str) {
    let Some((step, sync)) = p.get(0, HEADER_LEN).and_then(|h| ts_layout(&h)) else { return (None, "ts") };
    let mut pos = 0u64;
    while p.get(pos + sync, 1).is_some_and(|s| s[0] == 0x47) {
        pos += step;
    }
    (Some(pos), if step == 188 { "ts" } else { "m2ts" })
}

/// Length of an MPEG-1 or MPEG-2 pack header, from its marker bits.
fn mpg_pack_len(b: &[u8]) -> Option<u64> {
    if !b.starts_with(b"\0\0\x01\xBA") || b.len() < 14 {
        return None;
    }
    if b[4] & 0xC4 == 0x44 && b[6] & 4 != 0 && b[8] & 4 != 0 && b[9] & 1 != 0 && b[12] & 3 == 3 {
        Some(14 + (b[13] & 7) as u64)
    } else if b[4] & 0xF1 == 0x21 && b[6] & 1 != 0 && b[8] & 1 != 0 && b[9] & 0x80 != 0 && b[11] & 1 != 0 {
        Some(12)
    } else {
        None
    }
}

/// Packs and PES packets back to back until the end code or the first
/// thing that isn't a start code.
fn mpg_end(p: &mut Probe) -> Option<u64> {
    let (mut pos, mut packs) = (0u64, 0);
    while let Some(b) = p.get(pos, 16).filter(|b| b.starts_with(b"\0\0\x01")) {
        pos += match b[3] {
            0xB9 => return Some(pos + 4),
            0xBA => {
                packs += 1;
                match mpg_pack_len(&b) {
                    Some(n) => n,
                    None => break,
                }
            }
            0xBB.. => 6 + be16(&b, 4) as u64,
            _ => break,
        };
    }
    (packs > 0).then_some(pos)
}

const MP3_KBPS: [[u64; 15]; 2] = [
    [0, 32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320], // MPEG-1
    [0, 8, 16, 24, 32, 40, 48, 56, 64, 80, 96, 112, 128, 144, 160],     // MPEG-2/2.5
];

/// A Layer III frame header: its length and the bits every frame of the
/// file shares (version, layer, sample rate).
fn mp3_frame(h: &[u8]) -> Option<(u64, u32)> {
    let (ver, layer) = ((h[1] >> 3) & 3, (h[1] >> 1) & 3);
    let (bri, sri) = ((h[2] >> 4) as usize, ((h[2] >> 2) & 3) as usize);
    if h[0] != 0xFF || h[1] & 0xE0 != 0xE0 || ver == 1 || layer != 1 || bri == 0 || bri == 15 || sri == 3 {
        return None;
    }
    let rate = [44100, 48000, 32000][sri] >> [2, 0, 1, 0][ver as usize];
    let kbps = MP3_KBPS[(ver != 3) as usize][bri];
    let per = if ver == 3 { 144 } else { 72 };
    Some((per * kbps * 1000 / rate + ((h[2] >> 1) & 1) as u64, u32::from(h[1] & 0xFE) << 8 | u32::from(h[2] & 0x0C)))
}

/// Past the ID3v2 tag, then frame by frame while headers stay consistent,
/// plus an ID3v1 `TAG` trailer.
// ponytail: MP3s without an ID3v2 tag have no reliable header to find.
fn mp3_end(p: &mut Probe) -> Option<u64> {
    let h = p.get(0, 10)?;
    let tag = h[6..10].iter().fold(0u64, |v, &c| v << 7 | c as u64);
    let mut pos = 10 + tag + if h[5] & 0x10 != 0 { 10 } else { 0 };
    // Some taggers pad with zeros past the declared size.
    pos += p.get_upto(pos, 4096).iter().position(|&c| c != 0)? as u64;
    let (_, key) = mp3_frame(&p.get(pos, 4)?)?;
    while let Some((len, _)) = p.get(pos, 4).and_then(|f| mp3_frame(&f)).filter(|f| f.1 == key) {
        pos += len;
    }
    if p.get(pos, 3).is_some_and(|t| t == b"TAG") {
        pos += 128;
    }
    Some(pos)
}

/// Pages until every logical stream has hit end-of-stream (and no chained
/// stream starts right after). The first page's CRC must check out.
fn ogg_end(p: &mut Probe) -> (Option<u64>, &'static str) {
    let first = p.get_upto(0, 27 + 255 * 256);
    let opus = first.get(26).and_then(|&n| first.get(27 + n as usize..)).is_some_and(|d| d.starts_with(b"OpusHead"));
    let kind = if opus { "opus" } else { "ogg" };
    let end = (|| {
        let (mut pos, mut open) = (0u64, Vec::new());
        loop {
            let h = p.get(pos, 27).filter(|h| h.starts_with(b"OggS") && h[4] == 0)?;
            let segs = p.get(pos + 27, h[26] as usize)?;
            let len = 27 + segs.len() as u64 + segs.iter().map(|&s| s as u64).sum::<u64>();
            if pos == 0 {
                let mut page = p.get(0, len as usize)?;
                page[22..26].fill(0);
                if ogg_crc(&page) != le32(&h, 22) {
                    return None;
                }
            }
            let serial = le32(&h, 14);
            if h[5] & 2 != 0 {
                open.push(serial);
            }
            pos += len;
            if h[5] & 4 != 0 {
                open.retain(|&s| s != serial);
                if open.is_empty() && !p.get(pos, 6).is_some_and(|n| n.starts_with(b"OggS") && n[5] & 2 != 0) {
                    return Some(pos);
                }
            }
        }
    })();
    (end, kind)
}

/// Ogg's CRC-32 (polynomial 0x04C11DB7, not reflected — unlike zip's).
pub(super) fn ogg_crc(b: &[u8]) -> u32 {
    let mut crc = 0u32;
    for &c in b {
        crc ^= (c as u32) << 24;
        for _ in 0..8 {
            crc = if crc & 0x8000_0000 != 0 { (crc << 1) ^ 0x04C1_1DB7 } else { crc << 1 };
        }
    }
    crc
}

fn midi_end(p: &mut Probe) -> Option<u64> {
    let tracks = be16(&p.get(10, 2)?, 0);
    let mut pos = 14u64;
    for _ in 0..tracks {
        let c = p.get(pos, 8).filter(|c| c.starts_with(b"MTrk"))?;
        pos += 8 + be32(&c, 4) as u64;
    }
    Some(pos)
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

/// The stream footer: `YZ` preceded by a CRC over the backward size and a
/// copy of the header's flags.
fn xz_end(p: &mut Probe) -> Option<u64> {
    let flags = p.get(6, 2)?;
    let mut from = 12 + 10;
    loop {
        let at = p.find(from, b"YZ")?;
        from = at + 2;
        let f = p.get(at - 10, 12)?;
        if f[8..10] == flags[..] && crc32(&f[4..10]) == le32(&f, 0) {
            return Some(at + 2);
        }
    }
}

/// Frame header length (after the magic) from its descriptor byte.
fn zstd_header_len(fhd: u8) -> u64 {
    let single = fhd & 0x20 != 0;
    1 + !single as u64 + [0, 1, 2, 4][(fhd & 3) as usize] + [single as u64, 2, 4, 8][(fhd >> 6) as usize]
}

fn zstd_block(h: &[u8]) -> Option<(bool, u64)> {
    let v = h[0] as u32 | (h[1] as u32) << 8 | (h[2] as u32) << 16;
    let (typ, size) = ((v >> 1) & 3, (v >> 3) as u64);
    // Blocks never exceed 128 KiB; type 3 is reserved.
    (typ != 3 && size <= 128 * KIB).then_some((v & 1 != 0, 3 + if typ == 1 { 1 } else { size }))
}

fn zstd_first_block_ok(b: &[u8]) -> bool {
    b[4] & 0x08 == 0 && zstd_block(&b[4 + zstd_header_len(b[4]) as usize..]).is_some()
}

/// Frames (and skippable frames) back to back, each walked block by block.
fn zstd_end(p: &mut Probe) -> Option<u64> {
    let (mut pos, mut frames) = (0u64, 0);
    while let Some(m) = p.get(pos, 5) {
        let magic = le32(&m, 0);
        if magic & 0xFFFF_FFF0 == 0x184D_2A50 {
            pos += 8 + le32(&p.get(pos + 4, 4)?, 0) as u64;
            continue;
        }
        if magic != 0xFD2F_B528 || m[4] & 0x08 != 0 {
            break;
        }
        let mut q = pos + 4 + zstd_header_len(m[4]);
        loop {
            let (last, len) = zstd_block(&p.get(q, 3)?)?;
            q += len;
            if last {
                break;
            }
        }
        pos = q + if m[4] & 4 != 0 { 4 } else { 0 };
        frames += 1;
    }
    (frames > 0).then_some(pos)
}

fn tar_checksum_ok(h: &[u8]) -> bool {
    let sum: u64 = h.iter().enumerate().map(|(i, &c)| if (148..156).contains(&i) { 32 } else { c as u64 }).sum();
    tar_number(&h[148..156]) == Some(sum)
}

/// Octal ASCII, or GNU base-256 when the high bit is set.
fn tar_number(f: &[u8]) -> Option<u64> {
    if f[0] & 0x80 != 0 {
        return Some(be64(f, f.len() - 8));
    }
    let digits: Vec<u8> =
        f.iter().copied().skip_while(|&c| c == b' ').take_while(|c| (b'0'..=b'7').contains(c)).collect();
    (!digits.is_empty()).then(|| digits.iter().fold(0, |v, &c| v * 8 + (c - b'0') as u64))
}

/// Headers and their data until the two zero blocks that end the archive.
fn tar_end(p: &mut Probe) -> Option<u64> {
    let mut pos = 0u64;
    loop {
        let h = p.get(pos, 512)?;
        if h.iter().all(|&c| c == 0) {
            return p.get(pos + 512, 512)?.iter().all(|&c| c == 0).then_some(pos + 1024);
        }
        if !tar_checksum_ok(&h) {
            return None;
        }
        pos += 512 + tar_number(&h[124..136])?.div_ceil(512) * 512;
    }
}

/// Members until the next 60-byte header doesn't end in "`\n".
fn ar_end(p: &mut Probe) -> (Option<u64>, &'static str) {
    let (mut pos, mut members, mut kind) = (8u64, 0, "a");
    while let Some(h) = p.get(pos, 60).filter(|h| &h[58..60] == b"`\n") {
        let Some(size) = std::str::from_utf8(&h[48..58]).ok().and_then(|s| s.trim_end().parse::<u64>().ok()) else {
            break;
        };
        if members == 0 && h.starts_with(b"debian-binary") {
            kind = "deb";
        }
        pos += 60 + size + size % 2;
        members += 1;
    }
    ((members > 0).then_some(pos), kind)
}

/// Where the PE header sits, if `MZ` really leads to one.
fn pe_offset(b: &[u8]) -> Option<usize> {
    let at = le32(b, 0x3C) as usize;
    (b.starts_with(b"MZ")
        && (0x40..=HEADER_LEN - 26).contains(&at)
        && &b[at..at + 4] == b"PE\0\0"
        && matches!(le16(b, at + 24), 0x10B | 0x20B))
    .then_some(at)
}

/// The furthest section's raw data, or the Authenticode signature (always
/// appended last, so signed installers keep their payload). Unsigned
/// overlays (installer payloads) are not covered by any header.
fn pe_end(p: &mut Probe) -> (Option<u64>, &'static str) {
    let Some(h) = p.get(0, HEADER_LEN) else { return (None, "exe") };
    let Some(at) = pe_offset(&h) else { return (None, "exe") };
    let kind = if le16(&h, at + 22) & 0x2000 != 0 { "dll" } else { "exe" };
    let end = (|| {
        let (sections, opt_size) = (le16(&h, at + 6) as u64, le16(&h, at + 20) as u64);
        if !(1..=96).contains(&sections) {
            return None;
        }
        let opt = p.get(at as u64 + 24, opt_size as usize)?;
        let dirs = if le16(&opt, 0) == 0x20B { 112 } else { 96 };
        let mut end = le32(opt.get(..64)?, 60) as u64; // SizeOfHeaders
        if opt.len() >= dirs + 40 && le32(&opt, dirs - 4) > 4 && le32(&opt, dirs + 36) > 0 {
            end = end.max(le32(&opt, dirs + 32) as u64 + le32(&opt, dirs + 36) as u64);
        }
        for s in p.get(at as u64 + 24 + opt_size, 40 * sections as usize)?.chunks(40) {
            if le32(s, 16) > 0 {
                end = end.max(le32(s, 20) as u64 + le32(s, 16) as u64);
            }
        }
        Some(end)
    })();
    (end, kind)
}

/// The furthest of the section header table, any section's data and any
/// segment's file data.
fn elf_end(p: &mut Probe) -> Option<u64> {
    let h = p.get(0, 64)?;
    let (wide, le) = (h[4] == 2, h[5] == 1);
    let w = if wide { 8 } else { 4 };
    let (phoff, shoff) = (uint(&h, 24 + w, w, le), uint(&h, 24 + 2 * w, w, le));
    let f = 24 + 3 * w + 4; // e_ehsize
    let (ehsize, phent, phnum, shent, shnum) = (
        uint(&h, f, 2, le),
        uint(&h, f + 2, 2, le),
        uint(&h, f + 4, 2, le),
        uint(&h, f + 6, 2, le),
        uint(&h, f + 8, 2, le),
    );
    if phent > 256 || shent > 256 {
        return None;
    }
    let mut end = ehsize.max(phoff.checked_add(phent * phnum)?).max(shoff.checked_add(shent * shnum)?);
    if shnum > 0 && shent >= if wide { 64 } else { 40 } {
        for s in p.get(shoff, (shent * shnum) as usize)?.chunks(shent as usize) {
            if uint(s, 4, 4, le) != 8 {
                // not SHT_NOBITS (.bss takes no file space)
                end = end.max(uint(s, 8 + 2 * w, w, le).checked_add(uint(s, 8 + 3 * w, w, le))?);
            }
        }
    }
    if phnum > 0 && phent >= if wide { 56 } else { 32 } {
        for s in p.get(phoff, (phent * phnum) as usize)?.chunks(phent as usize) {
            let (off, filesz) =
                if wide { (uint(s, 8, 8, le), uint(s, 32, 8, le)) } else { (uint(s, 4, 4, le), uint(s, 16, 4, le)) };
            end = end.max(off.checked_add(filesz)?);
        }
    }
    // Linkers put the section header table last. Anything claiming to end
    // past it means the table read here is another file's data (this one
    // is fragmented): better an unknown end than a giant bogus one.
    let sht_end = shoff + shent * shnum;
    if shnum > 0 && end > sht_end {
        return None;
    }
    Some(end)
}

/// The header's ibFileEof (8 bytes in Unicode files, 4 in ANSI ones).
fn pst_end(p: &mut Probe) -> (Option<u64>, &'static str) {
    let Some(h) = p.get(0, 0xC0) else { return (None, "pst") };
    let kind = if &h[8..10] == b"SO" { "ost" } else { "pst" };
    let end = if le16(&h, 10) >= 23 { le64(&h, 0xB8) } else { le32(&h, 0xA8) as u64 };
    (Some(end), kind)
}

/// Header, then each optional part its flags announce, then extra data
/// blocks until the zero-size terminator.
fn lnk_end(p: &mut Probe) -> Option<u64> {
    let flags = le32(&p.get(20, 4)?, 0);
    let mut pos = 76u64;
    if flags & 1 != 0 {
        pos += 2 + le16(&p.get(pos, 2)?, 0) as u64;
    }
    if flags & 2 != 0 {
        pos += le32(&p.get(pos, 4)?, 0) as u64;
    }
    let unit = if flags & 0x80 != 0 { 2 } else { 1 };
    for bit in [0x4, 0x8, 0x10, 0x20, 0x40] {
        if flags & bit != 0 {
            pos += 2 + unit * le16(&p.get(pos, 2)?, 0) as u64;
        }
    }
    loop {
        let size = le32(&p.get(pos, 4)?, 0) as u64;
        if size < 4 {
            return Some(pos + 4);
        }
        pos += size;
    }
}

/// Version fields plus the header's XOR checksum.
fn regf_ok(b: &[u8]) -> bool {
    b.starts_with(b"regf")
        && le32(b, 0x14) == 1
        && le32(b, 0x1C) == 0
        && le32(b, 0x20) == 1
        && (0..127).fold(0u32, |x, i| x ^ le32(b, 4 * i)) == le32(b, 0x1FC)
}

fn regf_end(p: &mut Probe) -> Option<u64> {
    let size = le32(&p.get(0x28, 4)?, 0) as u64;
    p.get(4096, 4).filter(|h| h == b"hbin").map(|_| 4096 + size)
}

/// 64 KiB chunks after the 4 KiB header, as long as they carry the magic.
fn evtx_end(p: &mut Probe) -> Option<u64> {
    let mut n = 0u64;
    while p.get(4096 + n * 64 * KIB, 8).is_some_and(|c| c == b"ElfChnk\0") {
        n += 1;
    }
    (n > 0).then_some(4096 + n * 64 * KIB)
}

/// The table directory's search fields are derived from the table count,
/// which makes a 4-byte version tag safe to match on.
fn font_ok(b: &[u8]) -> bool {
    let n = be16(b, 4) as u32;
    let pow = if n == 0 { 0 } else { 1 << (31 - n.leading_zeros()) };
    matches!(&b[..4], b"\0\x01\0\0" | b"OTTO" | b"true")
        && (1..=64).contains(&n)
        && be16(b, 6) as u32 == 16 * pow
        && be16(b, 8) as u32 == pow.trailing_zeros()
        && be16(b, 10) as u32 == 16 * n - 16 * pow
}

fn font_end(p: &mut Probe) -> (Option<u64>, &'static str) {
    let Some(h) = p.get(0, 12) else { return (None, "ttf") };
    let kind = if h.starts_with(b"OTTO") { "otf" } else { "ttf" };
    let n = be16(&h, 4) as u64;
    let end = p.get(12, 16 * n as usize).map(|d| {
        // Tables are 4-byte aligned, the last one padded too.
        d.chunks(16).map(|t| (be32(t, 8) as u64 + be32(t, 12) as u64 + 3) & !3).fold(12 + 16 * n, u64::max)
    });
    (end, kind)
}

fn woff_end(p: &mut Probe) -> (Option<u64>, &'static str) {
    let Some(h) = p.get(0, 12) else { return (None, "woff") };
    (Some(be32(&h, 8) as u64), if h.starts_with(b"wOF2") { "woff2" } else { "woff" })
}

/// File blocks (code, size, old pointer, SDNA index, count) until `ENDB`.
fn blend_end(p: &mut Probe) -> Option<u64> {
    let h = p.get(0, 12)?;
    let (ptr, le) = (if h[7] == b'-' { 8 } else { 4 }, h[8] == b'v');
    let mut pos = 12u64;
    loop {
        let b = p.get(pos, 16 + ptr)?;
        if !b[..4].iter().all(|&c| c.is_ascii_alphanumeric() || c == 0) {
            return None;
        }
        pos += 16 + ptr as u64 + uint(&b, 4, 4, le);
        if &b[..4] == b"ENDB" {
            return Some(pos);
        }
    }
}

fn uint(b: &[u8], o: usize, n: usize, le: bool) -> u64 {
    let f = |v: u64, &c: &u8| v << 8 | c as u64;
    if le {
        b[o..o + n].iter().rev().fold(0, f)
    } else {
        b[o..o + n].iter().fold(0, f)
    }
}
fn be16(b: &[u8], o: usize) -> u16 {
    u16::from_be_bytes([b[o], b[o + 1]])
}
fn be24(b: &[u8], o: usize) -> u32 {
    u32::from_be_bytes([0, b[o], b[o + 1], b[o + 2]])
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
