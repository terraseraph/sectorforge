use super::*;
use crate::block_device::FakeBlockDevice;
use crate::partitions::crc32;
use crate::reader::RangeSet;

fn be32(n: u32) -> [u8; 4] {
    n.to_be_bytes()
}

fn jpg(body_len: usize) -> Vec<u8> {
    let mut f = vec![0xFF, 0xD8];
    // APP0
    f.extend_from_slice(&[0xFF, 0xE0, 0x00, 0x10]);
    f.extend_from_slice(b"JFIF\0\x01\x01\0\0\x01\0\x01\0\0");
    // APP1 holding a thumbnail with its own SOI/EOI: must not end the file.
    let thumb = [0xFF, 0xD8, 0xFF, 0xDB, 0x00, 0x02, 0xFF, 0xD9];
    let app1_len = (2 + 6 + thumb.len()) as u16;
    f.extend_from_slice(&[0xFF, 0xE1]);
    f.extend_from_slice(&app1_len.to_be_bytes());
    f.extend_from_slice(b"Exif\0\0");
    f.extend_from_slice(&thumb);
    // DQT
    f.extend_from_slice(&[0xFF, 0xDB, 0x00, 0x43]);
    f.extend(std::iter::repeat(7).take(0x41));
    // SOS + entropy data with stuffed FF00 and a restart marker
    f.extend_from_slice(&[0xFF, 0xDA, 0x00, 0x08, 1, 1, 0, 0, 0x3F, 0]);
    for i in 0..body_len {
        f.push((i % 200) as u8 + 1);
        if i % 97 == 0 {
            f.extend_from_slice(&[0xFF, 0x00]);
        }
        if i % 1001 == 0 {
            f.extend_from_slice(&[0xFF, 0xD3]);
        }
    }
    f.extend_from_slice(&[0xFF, 0xD9]);
    f
}

fn png_chunk(typ: &[u8; 4], data: &[u8]) -> Vec<u8> {
    let mut c = be32(data.len() as u32).to_vec();
    c.extend_from_slice(typ);
    c.extend_from_slice(data);
    c.extend_from_slice(&[0; 4]);
    c
}

fn png() -> Vec<u8> {
    let mut f = b"\x89PNG\r\n\x1a\n".to_vec();
    f.extend(png_chunk(b"IHDR", &[0, 0, 0, 1, 0, 0, 0, 1, 8, 2, 0, 0, 0]));
    f.extend(png_chunk(b"IDAT", &vec![0x42; 5000]));
    f.extend(png_chunk(b"IEND", &[]));
    f
}

fn gif() -> Vec<u8> {
    let mut f = b"GIF89a".to_vec();
    f.extend_from_slice(&[1, 0, 1, 0, 0x81, 0, 0]); // GCT, 4 entries
    f.extend_from_slice(&[0; 12]);
    f.extend_from_slice(&[0x21, 0xF9, 4, 0, 0, 0, 0, 0]); // GCE
    f.extend_from_slice(&[0x2C, 0, 0, 0, 0, 1, 0, 1, 0, 0]);
    f.push(2); // LZW min code size
    f.extend_from_slice(&[255]);
    f.extend(std::iter::repeat(0x3B).take(255)); // trailer bytes inside data
    f.extend_from_slice(&[3, 1, 2, 3, 0]);
    f.push(0x3B);
    f
}

fn pdf() -> Vec<u8> {
    let mut f = b"%PDF-1.4\n1 0 obj << >> endobj\nxref\n0 1\ntrailer << >>\nstartxref\n9\n%%EOF\n".to_vec();
    // An incremental update after the first %%EOF.
    f.extend_from_slice(b"2 0 obj << /Type /Page >> endobj\nxref\n0 1\ntrailer << >>\nstartxref\n77\n%%EOF\r\n");
    f
}

fn zip(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut f = Vec::new();
    let mut cd = Vec::new();
    for (name, data) in entries {
        let off = f.len() as u32;
        f.extend_from_slice(b"PK\x03\x04\x14\0\0\0\0\0\0\0\0\0");
        f.extend_from_slice(&crc32(data).to_le_bytes());
        f.extend_from_slice(&(data.len() as u32).to_le_bytes());
        f.extend_from_slice(&(data.len() as u32).to_le_bytes());
        f.extend_from_slice(&(name.len() as u16).to_le_bytes());
        f.extend_from_slice(&[0, 0]);
        f.extend_from_slice(name.as_bytes());
        f.extend_from_slice(data);
        cd.extend_from_slice(b"PK\x01\x02\x14\0\x14\0\0\0\0\0\0\0\0\0");
        cd.extend_from_slice(&crc32(data).to_le_bytes());
        cd.extend_from_slice(&(data.len() as u32).to_le_bytes());
        cd.extend_from_slice(&(data.len() as u32).to_le_bytes());
        cd.extend_from_slice(&(name.len() as u16).to_le_bytes());
        cd.extend_from_slice(&[0; 12]);
        cd.extend_from_slice(&off.to_le_bytes());
        cd.extend_from_slice(name.as_bytes());
    }
    let cd_off = f.len() as u32;
    f.extend_from_slice(&cd);
    f.extend_from_slice(b"PK\x05\x06\0\0\0\0");
    f.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    f.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    f.extend_from_slice(&(cd.len() as u32).to_le_bytes());
    f.extend_from_slice(&cd_off.to_le_bytes());
    f.extend_from_slice(&3u16.to_le_bytes());
    f.extend_from_slice(b"hi!");
    f
}

fn mp4_box(typ: &[u8; 4], data: &[u8]) -> Vec<u8> {
    let mut b = be32(8 + data.len() as u32).to_vec();
    b.extend_from_slice(typ);
    b.extend_from_slice(data);
    b
}

fn mp4(brand: &[u8; 4]) -> Vec<u8> {
    let mut f = mp4_box(b"ftyp", &[brand.as_slice(), &[0, 0, 2, 0], b"isommp41"].concat());
    f.extend(mp4_box(b"free", &[0; 8]));
    f.extend(mp4_box(b"mdat", &vec![0x11; 70_000]));
    f.extend(mp4_box(b"moov", &mp4_box(b"mvhd", &[0; 100])));
    f
}

fn sqlite() -> Vec<u8> {
    let mut f = vec![0x33u8; 4096 * 3];
    f[..16].copy_from_slice(b"SQLite format 3\0");
    f[16..18].copy_from_slice(&4096u16.to_be_bytes());
    f[24..28].copy_from_slice(&be32(9));
    f[28..32].copy_from_slice(&be32(3));
    f[92..96].copy_from_slice(&be32(9));
    f
}

fn sevenz() -> Vec<u8> {
    let packed = vec![0x77u8; 3000];
    let next_header = vec![0x01u8, 0x04, 0x06, 0x00];
    let mut h = b"7z\xBC\xAF\x27\x1C\x00\x04".to_vec();
    let mut start = Vec::new();
    start.extend_from_slice(&(packed.len() as u64).to_le_bytes());
    start.extend_from_slice(&(next_header.len() as u64).to_le_bytes());
    start.extend_from_slice(&crc32(&next_header).to_le_bytes());
    h.extend_from_slice(&crc32(&start).to_le_bytes());
    h.extend_from_slice(&start);
    h.extend_from_slice(&packed);
    h.extend_from_slice(&next_header);
    h
}

fn rar4() -> Vec<u8> {
    let mut f = b"Rar!\x1A\x07\x00".to_vec();
    f.extend_from_slice(&[0, 0, 0x73, 0, 0, 13, 0, 0, 0, 0, 0, 0, 0]); // archive header
    let data = vec![0x55u8; 2500];
    let name = b"a.txt";
    let head_size = 32 + name.len() as u16;
    let mut fh = vec![0, 0, 0x74];
    fh.extend_from_slice(&0x8000u16.to_le_bytes());
    fh.extend_from_slice(&head_size.to_le_bytes());
    fh.extend_from_slice(&(data.len() as u32).to_le_bytes()); // PACK_SIZE
    fh.resize(26, 0);
    fh.extend_from_slice(&(name.len() as u16).to_le_bytes());
    fh.resize(32, 0);
    fh.extend_from_slice(name);
    f.extend(fh);
    f.extend(data);
    f.extend_from_slice(&[0, 0, 0x7B, 0, 0x40, 7, 0]); // end of archive
    f
}

fn rar5() -> Vec<u8> {
    let mut f = b"Rar!\x1A\x07\x01\x00".to_vec();
    f.extend_from_slice(&[0, 0, 0, 0, 3, 1, 0, 0]); // main: size 3, type 1, flags 0, archive flags 0
    let data = vec![0x66u8; 300];
    // file header: type 2, flags 2 (has data), data size 300 (vint ac 02), then 2 body bytes
    let body = [2u8, 2, 0xAC, 0x02, 0, 0];
    f.extend_from_slice(&[0, 0, 0, 0, body.len() as u8]);
    f.extend_from_slice(&body);
    f.extend(data);
    f.extend_from_slice(&[0, 0, 0, 0, 3, 5, 0, 0]); // end of archive
    f
}

struct Disk {
    data: Vec<u8>,
}

impl Disk {
    fn new(size: usize) -> Self {
        Self { data: vec![0x5A; size] }
    }
    fn put(&mut self, at: usize, bytes: &[u8]) {
        self.data[at..at + bytes.len()].copy_from_slice(bytes);
    }
}

fn run(disk: &Disk, opts: CarveOptions, bad: RangeSet) -> (Vec<CarvedFile>, tempfile::TempDir) {
    let mut dev = FakeBlockDevice::new(disk.data.clone(), 512);
    let mut reader = SafeReader::new(&mut dev, bad);
    let out = tempfile::tempdir().unwrap();
    let mut files = Vec::new();
    let summary = carve(&mut reader, out.path(), &opts, |_, _, _| {}, |f| files.push(f.clone())).unwrap();
    assert_eq!(summary.files_found as usize, files.len());
    (files, out)
}

fn all() -> CarveOptions {
    CarveOptions { types: vec![], start: 0, end: None }
}

#[test]
fn recovers_every_format_exactly() {
    let samples: Vec<(&str, Vec<u8>)> = vec![
        ("jpg", jpg(20_000)),
        ("png", png()),
        ("gif", gif()),
        ("pdf", pdf()),
        ("zip", zip(&[("a.txt", b"hello"), ("b.txt", b"world")])),
        ("docx", zip(&[("[Content_Types].xml", b"<x/>"), ("word/document.xml", b"<w/>")])),
        ("xlsx", zip(&[("[Content_Types].xml", b"<x/>"), ("xl/workbook.xml", b"<w/>")])),
        ("pptx", zip(&[("[Content_Types].xml", b"<x/>"), ("ppt/presentation.xml", b"<p/>")])),
        ("mp4", mp4(b"isom")),
        ("mov", mp4(b"qt  ")),
        ("sqlite", sqlite()),
        ("7z", sevenz()),
        ("rar", rar4()),
        ("rar", rar5()),
    ];
    let mut disk = Disk::new(4 * 1024 * 1024);
    let mut at = 512 * 3;
    let mut placed = Vec::new();
    for (t, bytes) in &samples {
        disk.put(at, bytes);
        placed.push((at as u64, *t, bytes.clone()));
        at = (at + bytes.len() + 512 * 5).div_ceil(512) * 512;
    }
    let (files, out) = run(&disk, all(), RangeSet::new());
    assert_eq!(files.len(), samples.len(), "{files:?}");
    for ((off, t, bytes), f) in placed.iter().zip(&files) {
        assert_eq!((f.offset, f.type_id, f.size), (*off, *t, bytes.len() as u64), "{f:?}");
        assert!(f.complete, "{f:?}");
        assert_eq!(f.name, format!("{t}/f{:07}.{t}", off / 512));
        assert_eq!(&std::fs::read(out.path().join(&f.name)).unwrap(), bytes, "{t}");
    }
}

#[test]
fn nested_zip_eocd_is_not_mistaken_for_the_outer_end() {
    let inner = zip(&[("inner.txt", b"inner data")]);
    let outer = zip(&[("inner.zip", &inner), ("z.txt", b"after")]);
    let mut disk = Disk::new(64 * 1024);
    disk.put(1024, &outer);
    let (files, _) = run(&disk, all(), RangeSet::new());
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].size, outer.len() as u64);
}

#[test]
fn missing_end_stops_at_the_next_header_and_is_incomplete() {
    let mut j = jpg(5000);
    j.truncate(j.len() - 2); // no EOI
    let mut disk = Disk::new(256 * 1024);
    disk.put(0, &j);
    disk.put(16 * 1024, &png());
    let (files, _) = run(&disk, all(), RangeSet::new());
    assert_eq!(files.len(), 2);
    assert_eq!((files[0].type_id, files[0].size, files[0].complete), ("jpg", 16 * 1024, false));
    assert_eq!((files[1].type_id, files[1].complete), ("png", true));
}

#[test]
fn map_holes_make_a_file_incomplete_but_it_is_still_written() {
    let p = png();
    let mut disk = Disk::new(64 * 1024);
    disk.put(4096, &p);
    let mut bad = RangeSet::new();
    bad.insert(4096 + 1024, 4096 + 1536);
    let (files, out) = run(&disk, all(), bad);
    assert_eq!(files.len(), 1);
    assert!(!files[0].complete);
    let got = std::fs::read(out.path().join(&files[0].name)).unwrap();
    assert_eq!(got.len(), p.len());
    assert!(got[1024..1536].iter().all(|&b| b == 0));
}

#[test]
fn read_errors_make_a_file_incomplete() {
    let p = png();
    let mut disk = Disk::new(64 * 1024);
    disk.put(4096, &p);
    let mut dev = FakeBlockDevice::new(disk.data.clone(), 512);
    dev.mark_bad(4096 + 2048, 4096 + 2560);
    let mut reader = SafeReader::new(&mut dev, RangeSet::new());
    let out = tempfile::tempdir().unwrap();
    let mut files = Vec::new();
    carve(&mut reader, out.path(), &all(), |_, _, _| {}, |f| files.push(f.clone())).unwrap();
    assert_eq!(files.len(), 1);
    assert!(!files[0].complete);
}

#[test]
fn does_not_carve_inside_a_claimed_file() {
    // A PNG stored (sector-aligned) inside a zip is part of the zip.
    let mut payload = vec![0u8; 512 - 30 - 7];
    payload.extend(png());
    let z = zip(&[("pic.png", &payload)]);
    let mut disk = Disk::new(64 * 1024);
    disk.put(0, &z);
    assert_eq!(&disk.data[512..520], b"\x89PNG\r\n\x1a\n");
    let (files, _) = run(&disk, all(), RangeSet::new());
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].type_id, "zip");
}

#[test]
fn honors_types_and_range() {
    let mut disk = Disk::new(256 * 1024);
    disk.put(0, &png());
    disk.put(64 * 1024, &gif());
    disk.put(128 * 1024, &png());
    let (files, _) = run(&disk, CarveOptions { types: vec!["png".into()], start: 0, end: None }, RangeSet::new());
    assert_eq!(files.iter().map(|f| f.offset).collect::<Vec<_>>(), vec![0, 128 * 1024]);
    let (files, _) = run(&disk, CarveOptions { types: vec![], start: 1, end: Some(128 * 1024) }, RangeSet::new());
    assert_eq!(files.iter().map(|f| f.type_id).collect::<Vec<_>>(), vec!["gif"]);
}

#[test]
fn docx_filter_skips_plain_zips() {
    let mut disk = Disk::new(64 * 1024);
    disk.put(0, &zip(&[("a.txt", b"x")]));
    disk.put(8192, &zip(&[("word/document.xml", b"<w/>")]));
    let (files, _) = run(&disk, CarveOptions { types: vec!["docx".into()], start: 0, end: None }, RangeSet::new());
    assert_eq!(files.len(), 1);
    assert_eq!((files[0].offset, files[0].type_id), (8192, "docx"));
}

#[test]
fn rejects_unknown_types() {
    assert!(validate_types(&["jpg".into(), "docx".into()]).is_ok());
    assert!(validate_types(&["exe".into()]).unwrap_err().contains("exe"));
}
