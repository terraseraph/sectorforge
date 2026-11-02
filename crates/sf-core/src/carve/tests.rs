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
    f.extend(std::iter::repeat_n(7, 0x41));
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
    f.extend(std::iter::repeat_n(0x3B, 255)); // trailer bytes inside data
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

fn set(f: &mut [u8], at: usize, b: &[u8]) {
    f[at..at + b.len()].copy_from_slice(b);
}

const OLE_FREE: u32 = 0xFFFF_FFFF;

fn ole_entry(name: &str, typ: u8, right: u32, child: u32, start: u32, size: u32) -> Vec<u8> {
    let mut e = vec![0u8; 128];
    let units: Vec<u8> = name.encode_utf16().chain([0]).flat_map(u16::to_le_bytes).collect();
    set(&mut e, 0, &units);
    set(&mut e, 64, &(units.len() as u16).to_le_bytes());
    e[66] = typ;
    set(&mut e, 68, &OLE_FREE.to_le_bytes());
    set(&mut e, 72, &right.to_le_bytes());
    set(&mut e, 76, &child.to_le_bytes());
    set(&mut e, 116, &start.to_le_bytes());
    set(&mut e, 120, &size.to_le_bytes());
    e
}

/// Header, FAT (sector 0), directory (sector 1), a 4-sector stream. A
/// nested "Workbook" inside an ObjectPool storage must not decide the type.
fn ole(stream: &str) -> Vec<u8> {
    let mut f = vec![0u8; 512 * 7];
    set(&mut f, 0, b"\xD0\xCF\x11\xE0\xA1\xB1\x1A\xE1");
    set(&mut f, 0x18, &[0x3E, 0, 3, 0, 0xFE, 0xFF, 9, 0, 6, 0]);
    set(&mut f, 0x2C, &1u32.to_le_bytes());
    set(&mut f, 0x30, &1u32.to_le_bytes());
    set(&mut f, 0x38, &4096u32.to_le_bytes());
    set(&mut f, 0x3C, &0xFFFF_FFFEu32.to_le_bytes());
    set(&mut f, 0x44, &0xFFFF_FFFEu32.to_le_bytes());
    f[0x50..0x200].fill(0xFF);
    f[512..1024].fill(0xFF);
    let fat: Vec<u8> =
        [0xFFFF_FFFDu32, 0xFFFF_FFFE, 3, 4, 5, 0xFFFF_FFFE].iter().flat_map(|e| e.to_le_bytes()).collect();
    set(&mut f, 512, &fat);
    let dir = [
        ole_entry("Root Entry", 5, OLE_FREE, 1, 0xFFFF_FFFE, 0),
        ole_entry(stream, 2, 2, OLE_FREE, 2, 2048),
        ole_entry("ObjectPool", 1, OLE_FREE, 3, 0, 0),
        ole_entry("Workbook", 2, OLE_FREE, OLE_FREE, 2, 100),
    ]
    .concat();
    set(&mut f, 1024, &dir);
    f[1536..].fill(0x11);
    f
}

fn rtf() -> Vec<u8> {
    b"{\\rtf1\\ansi{\\fonttbl{\\f0 Arial;}}\\{not a group\\} \\\\ text}".to_vec()
}

/// An embedded EPS's %%EOF must not end the outer document.
fn ps() -> Vec<u8> {
    b"%!PS-Adobe-3.0\n%%Pages: 1\n%%BeginDocument: fig.eps\n%!PS-Adobe-3.0 EPSF-3.0\nshowpage\n%%EOF\n%%EndDocument\nshowpage\n%%EOF\r\n"
        .to_vec()
}

fn djvu() -> Vec<u8> {
    let mut body = b"DJVUINFO\0\0\0\x0A".to_vec();
    body.extend([7; 10]);
    let mut f = b"AT&TFORM".to_vec();
    f.extend(be32(body.len() as u32));
    f.extend(body);
    f
}

fn chm() -> Vec<u8> {
    let mut f = vec![0x2E; 2000];
    set(&mut f, 0, b"ITSF\x03\0\0\0\x60\0\0\0");
    set(&mut f, 0x38, &0x60u64.to_le_bytes());
    set(&mut f, 0x60, &0x1FEu32.to_le_bytes());
    set(&mut f, 0x68, &2000u64.to_le_bytes());
    f
}

fn bmp() -> Vec<u8> {
    let mut f = vec![0x3D; 54 + 320];
    set(&mut f, 0, b"BM");
    set(&mut f, 2, &374u32.to_le_bytes());
    set(&mut f, 6, &[0; 4]);
    for (at, v) in [(10, 54u32), (14, 40), (18, 10), (22, 10), (30, 0), (34, 320)] {
        set(&mut f, at, &v.to_le_bytes());
    }
    set(&mut f, 26, &[1, 0, 24, 0]);
    f
}

fn ifd(entries: &[(u16, u16, u32, u32)]) -> Vec<u8> {
    let mut d = (entries.len() as u16).to_le_bytes().to_vec();
    for (tag, typ, count, val) in entries {
        d.extend(tag.to_le_bytes());
        d.extend(typ.to_le_bytes());
        d.extend(count.to_le_bytes());
        d.extend(val.to_le_bytes());
    }
    d.extend([0; 4]);
    d
}

/// IFD0 with the make and a strip, plus a SubIFD whose strip is the last
/// thing in the file (as in camera raws).
fn tiff(magic: &[u8; 4], make: &str) -> Vec<u8> {
    let make = format!("{make}\0");
    let make_at = 8 + 2 + 4 * 12 + 4;
    let ifd1 = (make_at + make.len() + 1) & !1;
    let s1 = ifd1 + 2 + 2 * 12 + 4;
    let s2 = s1 + 1000;
    let mut f = magic.to_vec();
    f.extend(8u32.to_le_bytes());
    f.extend(ifd(&[
        (271, 2, make.len() as u32, make_at as u32),
        (273, 4, 1, s1 as u32),
        (279, 4, 1, 1000),
        (330, 4, 1, ifd1 as u32),
    ]));
    f.extend(make.as_bytes());
    f.resize(ifd1, 0);
    f.extend(ifd(&[(273, 4, 1, s2 as u32), (279, 3, 1, 3000)]));
    f.resize(s2 + 3000, 0x6E);
    f
}

fn ico() -> Vec<u8> {
    let mut f = vec![0, 0, 1, 0, 2, 0];
    f.extend([16, 16, 0, 0, 1, 0, 32, 0]);
    f.extend(100u32.to_le_bytes());
    f.extend(38u32.to_le_bytes());
    f.extend([32, 32, 0, 0, 1, 0, 32, 0]);
    f.extend(200u32.to_le_bytes());
    f.extend(138u32.to_le_bytes());
    f.resize(338, 0x44);
    f
}

/// 3 channels x 2 rows, RLE: six 3-byte rows.
fn psd() -> Vec<u8> {
    let mut f = b"8BPS\0\x01\0\0\0\0\0\0\0\x03".to_vec();
    f.extend(be32(2));
    f.extend(be32(4));
    f.extend([0, 8, 0, 3]);
    f.extend(be32(0));
    f.extend(be32(4));
    f.extend([1, 2, 3, 4]);
    f.extend(be32(0));
    f.extend([0, 1]);
    for _ in 0..6 {
        f.extend([0, 3]);
    }
    f.extend([0x77; 18]);
    f
}

fn jp2() -> Vec<u8> {
    let mut f = b"\0\0\0\x0CjP  \r\n\x87\n".to_vec();
    f.extend(mp4_box(b"ftyp", b"jp2 \0\0\0\0jp2 "));
    f.extend(mp4_box(b"jp2h", &mp4_box(b"ihdr", &[0; 14])));
    f.extend(mp4_box(b"jp2c", &[0x33; 900]));
    f
}

fn riff(form: &[u8; 4], chunks: &[(&[u8; 4], usize)]) -> Vec<u8> {
    let mut body = form.to_vec();
    for (id, len) in chunks {
        body.extend(*id);
        body.extend((*len as u32).to_le_bytes());
        body.extend(vec![0x2A; *len + len % 2]);
    }
    let mut f = b"RIFF".to_vec();
    f.extend((body.len() as u32).to_le_bytes());
    f.extend(body);
    f
}

fn avi() -> Vec<u8> {
    let mut f = riff(b"AVI ", &[(b"LIST", 200), (b"idx1", 64)]);
    f.extend(riff(b"AVIX", &[(b"LIST", 500)]));
    f
}

fn mp3() -> Vec<u8> {
    let mut f = b"ID3\x03\0\0\0\0\0\x14".to_vec();
    f.extend([0; 20]);
    for _ in 0..5 {
        f.extend([0xFF, 0xFB, 0x90, 0x00]); // MPEG-1 Layer III, 128 kbps, 44.1 kHz: 417 bytes
        f.extend([0x21; 413]);
    }
    f.extend(b"TAG");
    f.extend([0x20; 125]);
    f
}

fn ogg_page(flags: u8, seq: u32, data: &[u8]) -> Vec<u8> {
    let mut p = b"OggS\0".to_vec();
    p.push(flags);
    p.extend(0u64.to_le_bytes());
    p.extend(7u32.to_le_bytes());
    p.extend(seq.to_le_bytes());
    p.extend([0; 4]);
    p.extend([1, data.len() as u8]);
    p.extend(data);
    let crc = formats::ogg_crc(&p);
    set(&mut p, 22, &crc.to_le_bytes());
    p
}

fn opus() -> Vec<u8> {
    [
        ogg_page(2, 0, b"OpusHead\x01\x02\0\0\x80\xBB\0\0\0\0\0"),
        ogg_page(0, 1, &[0x55; 200]),
        ogg_page(4, 2, &[0x66; 100]),
    ]
    .concat()
}

fn aiff() -> Vec<u8> {
    let mut body = b"AIFFCOMM".to_vec();
    body.extend(be32(18));
    body.extend([0; 18]);
    body.extend(b"SSND");
    body.extend(be32(1000));
    body.extend([0x19; 1000]);
    let mut f = b"FORM".to_vec();
    f.extend(be32(body.len() as u32));
    f.extend(body);
    f
}

fn asf() -> Vec<u8> {
    let mut props = b"\xA1\xDC\xAB\x8C\x47\xA9\xCF\x11\x8E\xE4\x00\xC0\x0C\x20\x53\x65".to_vec();
    props.extend(104u64.to_le_bytes());
    props.extend([0; 16]);
    props.extend(3000u64.to_le_bytes());
    props.resize(104, 0);
    let mut stream = b"\x91\x07\xDC\xB7\xB7\xA9\xCF\x11\x8E\xE6\x00\xC0\x0C\x20\x53\x65".to_vec();
    stream.extend(78u64.to_le_bytes());
    stream.extend(b"\xC0\xEF\x19\xBC\x4D\x5B\xCF\x11\xA8\xFD\x00\x80\x5F\x5C\x44\x2B");
    stream.resize(78, 0);
    let mut f = b"\x30\x26\xB2\x75\x8E\x66\xCF\x11\xA6\xD9\x00\xAA\x00\x62\xCE\x6C".to_vec();
    f.extend(((30 + props.len() + stream.len()) as u64).to_le_bytes());
    f.extend(2u32.to_le_bytes());
    f.extend([1, 2]);
    f.extend(props);
    f.extend(stream);
    f.resize(3000, 0x77);
    f
}

fn midi() -> Vec<u8> {
    let mut f = b"MThd\0\0\0\x06\0\x01\0\x02\x01\xE0".to_vec();
    for _ in 0..2 {
        f.extend(b"MTrk\0\0\0\x0A");
        f.extend([0x90; 10]);
    }
    f
}

fn mkv(doctype: &str) -> Vec<u8> {
    let mut hdr = vec![0x42, 0x82, 0x80 | doctype.len() as u8];
    hdr.extend(doctype.as_bytes());
    let mut f = vec![0x1A, 0x45, 0xDF, 0xA3, 0x80 | hdr.len() as u8];
    f.extend(hdr);
    f.extend([0x18, 0x53, 0x80, 0x67, 0x01, 0, 0, 0, 0, 0, 0x13, 0x88]); // 8-byte size: 5000
    f.extend([0x44; 5000]);
    f
}

fn flv() -> Vec<u8> {
    let mut f = b"FLV\x01\x05\0\0\0\x09\0\0\0\0".to_vec();
    for typ in [18, 9, 8] {
        f.extend([typ, 0, 0, 100, 0, 0, 0, 0, 0, 0, 0]);
        f.extend([0x31; 100]);
        f.extend(be32(111));
    }
    f
}

fn ts(step: usize) -> Vec<u8> {
    (0..20)
        .flat_map(|_| {
            let mut p = vec![0x10u8; step];
            p[step - 188] = 0x47;
            p
        })
        .collect()
}

fn mpg() -> Vec<u8> {
    let mut f = Vec::new();
    for _ in 0..3 {
        f.extend([0, 0, 1, 0xBA, 0x44, 0, 4, 0, 4, 1, 0x01, 0x89, 0xC3, 0xF8]);
        f.extend([0, 0, 1, 0xE0, 0, 200]);
        f.extend([0x0F; 200]);
    }
    f.extend([0, 0, 1, 0xB9]);
    f
}

fn xz() -> Vec<u8> {
    let mut f = b"\xFD7zXZ\0\0\x01".to_vec();
    f.extend(crc32(&[0, 1]).to_le_bytes());
    f.extend([0x42; 1000]);
    let back = [3, 0, 0, 0, 0, 1];
    f.extend(crc32(&back).to_le_bytes());
    f.extend(back);
    f.extend(b"YZ");
    f
}

/// A raw-block frame, a skippable frame, then an RLE-block frame with a
/// checksum.
fn zstd() -> Vec<u8> {
    let mut f = vec![0x28, 0xB5, 0x2F, 0xFD, 0x20, 100, 0x21, 0x03, 0x00];
    f.extend([0x61; 100]);
    f.extend([0x50, 0x2A, 0x4D, 0x18, 16, 0, 0, 0]);
    f.extend([0; 16]);
    f.extend([0x28, 0xB5, 0x2F, 0xFD, 0x24, 0x88, 0x43, 0x04, 0x00, 0x7A]);
    f.extend([1, 2, 3, 4]);
    f
}

fn tar_header(name: &str, size: usize, typ: u8) -> Vec<u8> {
    let mut h = vec![0u8; 512];
    set(&mut h, 0, name.as_bytes());
    set(&mut h, 100, b"0000644\0");
    set(&mut h, 124, format!("{size:011o}\0").as_bytes());
    set(&mut h, 136, b"00000000000\0");
    h[156] = typ;
    set(&mut h, 257, b"ustar\x0000");
    h[148..156].fill(b' ');
    let sum: u32 = h.iter().map(|&c| c as u32).sum();
    set(&mut h, 148, format!("{sum:06o}\0 ").as_bytes());
    h
}

fn tar() -> Vec<u8> {
    let mut f = tar_header("a.txt", 700, b'0');
    f.extend([0x41; 700]);
    f.resize(512 + 1024, 0);
    f.extend(tar_header("b/", 0, b'5'));
    f.extend([0; 1024]);
    f
}

fn cab() -> Vec<u8> {
    let mut f = vec![0x2B; 2000];
    set(&mut f, 0, b"MSCF\0\0\0\0");
    set(&mut f, 8, &2000u32.to_le_bytes());
    set(&mut f, 12, &[0; 4]);
    set(&mut f, 16, &44u32.to_le_bytes());
    set(&mut f, 20, &[0, 0, 0, 0, 3, 1]);
    f
}

fn ar_member(name: &str, data: &[u8]) -> Vec<u8> {
    let mut m = format!("{name:<16}{:<12}{:<6}{:<6}{:<8}{:<10}`\n", 0, 0, 0, 100644, data.len()).into_bytes();
    m.extend(data);
    if data.len() % 2 == 1 {
        m.push(b'\n');
    }
    m
}

fn deb() -> Vec<u8> {
    [b"!<arch>\n".to_vec(), ar_member("debian-binary", b"2.0\n"), ar_member("control.tar.gz", &[0x1F; 101])].concat()
}

/// Two sections and an Authenticode signature appended after them.
fn pe(dll: bool) -> Vec<u8> {
    let mut f = vec![0u8; 0xB00];
    set(&mut f, 0, b"MZ");
    set(&mut f, 0x3C, &0x80u32.to_le_bytes());
    set(&mut f, 0x80, b"PE\0\0\x4C\x01\x02\0");
    set(&mut f, 0x94, &224u16.to_le_bytes());
    set(&mut f, 0x96, &(0x0102u16 | if dll { 0x2000 } else { 0 }).to_le_bytes());
    set(&mut f, 0x98, &0x10Bu16.to_le_bytes());
    set(&mut f, 0x98 + 60, &0x400u32.to_le_bytes());
    set(&mut f, 0x98 + 92, &16u32.to_le_bytes());
    set(&mut f, 0x98 + 128, &0xA00u32.to_le_bytes());
    set(&mut f, 0x98 + 132, &0x100u32.to_le_bytes());
    for (i, (size, ptr)) in [(0x200u32, 0x400u32), (0x400, 0x600)].iter().enumerate() {
        set(&mut f, 0x178 + 40 * i + 16, &size.to_le_bytes());
        set(&mut f, 0x178 + 40 * i + 20, &ptr.to_le_bytes());
    }
    f
}

fn elf() -> Vec<u8> {
    let mut f = vec![0u8; 0x7C0];
    set(&mut f, 0, b"\x7FELF\x02\x01\x01");
    set(&mut f, 16, &[2, 0, 0x3E, 0, 1, 0, 0, 0]);
    set(&mut f, 32, &64u64.to_le_bytes());
    set(&mut f, 40, &0x700u64.to_le_bytes());
    set(&mut f, 52, &[64, 0, 56, 0, 1, 0, 64, 0, 3, 0]);
    set(&mut f, 64, &1u32.to_le_bytes());
    set(&mut f, 64 + 32, &0x400u64.to_le_bytes());
    // .text, then a .bss that takes no file space.
    for (i, (typ, off, size)) in [(1u32, 0x400u64, 0x300u64), (8, 0x700, 0x10000)].iter().enumerate() {
        let s = 0x740 + 64 * i;
        set(&mut f, s + 4, &typ.to_le_bytes());
        set(&mut f, s + 24, &off.to_le_bytes());
        set(&mut f, s + 32, &size.to_le_bytes());
    }
    f
}

fn pst() -> Vec<u8> {
    let mut f = vec![0x3C; 8192];
    set(&mut f, 0, b"!BDN\0\0\0\0SM\x17\0");
    set(&mut f, 0xB8, &8192u64.to_le_bytes());
    f
}

fn lnk() -> Vec<u8> {
    let mut f = vec![0u8; 76];
    set(&mut f, 0, &0x4Cu32.to_le_bytes());
    set(&mut f, 4, b"\x01\x14\x02\x00\x00\x00\x00\x00\xC0\x00\x00\x00\x00\x00\x00\x46");
    set(&mut f, 20, &0x85u32.to_le_bytes()); // ID list, name, Unicode
    f.extend([20, 0]);
    f.extend([0x09; 20]);
    f.extend([5, 0]);
    f.extend("hello".encode_utf16().flat_map(u16::to_le_bytes));
    f.extend(12u32.to_le_bytes());
    f.extend([0x0A; 8]);
    f.extend([0; 4]);
    f
}

fn regf() -> Vec<u8> {
    let mut f = vec![0u8; 4096 + 8192];
    set(&mut f, 0, b"regf\x01\0\0\0\x01\0\0\0");
    for (at, v) in [(0x14, 1u32), (0x18, 5), (0x1C, 0), (0x20, 1), (0x24, 0x20), (0x28, 8192)] {
        set(&mut f, at, &v.to_le_bytes());
    }
    let x = (0..127).fold(0u32, |x, i| x ^ u32::from_le_bytes(f[4 * i..4 * i + 4].try_into().unwrap()));
    set(&mut f, 0x1FC, &x.to_le_bytes());
    set(&mut f, 4096, b"hbin");
    f
}

fn evtx() -> Vec<u8> {
    let mut f = vec![0u8; 4096 + 2 * 65536];
    set(&mut f, 0, b"ElfFile\0");
    set(&mut f, 16, &1u64.to_le_bytes());
    set(&mut f, 32, &[128, 0, 0, 0, 1, 0, 3, 0, 0, 0x10, 2, 0]);
    let crc = crc32(&f[..120]);
    set(&mut f, 124, &crc.to_le_bytes());
    set(&mut f, 4096, b"ElfChnk\0");
    set(&mut f, 4096 + 65536, b"ElfChnk\0");
    f
}

fn ttf() -> Vec<u8> {
    let mut f = vec![0u8; 200];
    set(&mut f, 0, &[0, 1, 0, 0, 0, 2, 0, 32, 0, 1, 0, 0]);
    set(&mut f, 12, b"glyf\0\0\0\0\0\0\0\x2C\0\0\0\x64");
    set(&mut f, 28, b"head\0\0\0\0\0\0\0\x90\0\0\0\x36");
    f
}

fn woff() -> Vec<u8> {
    let mut f = b"wOFF\0\x01\0\0".to_vec();
    f.extend(be32(500));
    f.extend([0, 1, 0, 0]);
    f.resize(500, 0x56);
    f
}

fn blend() -> Vec<u8> {
    let mut f = b"BLENDER-v300".to_vec();
    for (code, size) in [(b"REND", 16usize), (b"DNA1", 100), (b"ENDB", 0)] {
        f.extend(code);
        f.extend((size as u32).to_le_bytes());
        f.extend([0; 16]);
        f.extend(vec![0x3F; size]);
    }
    f
}

fn heif(brand: &[u8; 4]) -> Vec<u8> {
    let mut f = mp4_box(b"ftyp", &[brand.as_slice(), &[0; 4], b"mif1heic"].concat());
    f.extend(mp4_box(b"meta", &[0; 50]));
    f.extend(mp4_box(b"mdat", &[0x12; 3000]));
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
        ("doc", ole("WordDocument")),
        ("xls", ole("Workbook")),
        ("msg", ole("__substg1.0_0037001F")),
        ("rtf", rtf()),
        ("ps", ps()),
        ("djvu", djvu()),
        ("chm", chm()),
        ("odt", zip(&[("mimetype", b"application/vnd.oasis.opendocument.text"), ("content.xml", b"<x/>")])),
        ("ods", zip(&[("mimetype", b"application/vnd.oasis.opendocument.spreadsheet"), ("content.xml", b"<x/>")])),
        ("epub", zip(&[("mimetype", b"application/epub+zip"), ("OEBPS/a.xhtml", b"<x/>")])),
        ("jar", zip(&[("META-INF/MANIFEST.MF", b"x"), ("a.class", b"y")])),
        ("apk", zip(&[("META-INF/MANIFEST.MF", b"x"), ("AndroidManifest.xml", b"y")])),
        ("bmp", bmp()),
        ("tif", tiff(b"II*\0", "Acme")),
        ("nef", tiff(b"II*\0", "NIKON CORPORATION")),
        ("orf", tiff(b"IIRO", "OLYMPUS")),
        ("ico", ico()),
        ("psd", psd()),
        ("jp2", jp2()),
        ("webp", riff(b"WEBP", &[(b"VP8 ", 301)])),
        ("wav", riff(b"WAVE", &[(b"fmt ", 16), (b"data", 1000)])),
        ("avi", avi()),
        ("heic", heif(b"heic")),
        ("avif", heif(b"avif")),
        ("m4a", mp4(b"M4A ")),
        ("3gp", mp4(b"3gp5")),
        ("cr3", mp4(b"crx ")),
        ("mp3", mp3()),
        ("opus", opus()),
        ("aiff", aiff()),
        ("wmv", asf()),
        ("mid", midi()),
        ("mkv", mkv("matroska")),
        ("webm", mkv("webm")),
        ("flv", flv()),
        ("ts", ts(188)),
        ("m2ts", ts(192)),
        ("mpg", mpg()),
        ("xz", xz()),
        ("zst", zstd()),
        ("tar", tar()),
        ("cab", cab()),
        ("deb", deb()),
        ("exe", pe(false)),
        ("dll", pe(true)),
        ("elf", elf()),
        ("pst", pst()),
        ("lnk", lnk()),
        ("dat", regf()),
        ("evtx", evtx()),
        ("ttf", ttf()),
        ("woff", woff()),
        ("blend", blend()),
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
    assert!(validate_types(&["bogus".into()]).unwrap_err().contains("bogus"));
}

#[test]
fn header_only_formats_are_written_up_to_the_next_header() {
    let samples: Vec<(&str, Vec<u8>)> = vec![
        ("flac", [b"fLaC\0\0\0\x22".as_slice(), &[0x10; 34]].concat()),
        ("gz", b"\x1F\x8B\x08\0\0\0\0\0\0\x03".to_vec()),
        ("bz2", b"BZh91AY&SY".to_vec()),
        ("kdbx", b"\x03\xD9\xA2\x9A\x67\xFB\x4B\xB5\x01\0\x04\0".to_vec()),
        ("xcf", b"gimp xcf v011\0".to_vec()),
        ("amr", b"#!AMR\n".to_vec()),
        ("dwg", b"AC1032\0\0\0\0\0\x01".to_vec()),
        // Panasonic raw data has no stored length.
        ("rw2", tiff(b"IIU\0", "Panasonic")),
    ];
    for (t, head) in samples {
        let mut disk = Disk::new(64 * 1024);
        disk.put(0, &head);
        disk.put(8192, &png());
        let (files, _) = run(&disk, all(), RangeSet::new());
        assert_eq!(files.len(), 2, "{t}: {files:?}");
        assert_eq!((files[0].type_id, files[0].size, files[0].complete), (t, 8192, false), "{t}");
    }
}

#[test]
fn every_type_id_belongs_to_exactly_one_table_entry() {
    let ids: Vec<&str> = Kind::ALL.iter().flat_map(|k| k.type_ids().iter().copied()).collect();
    for t in TYPES {
        assert!(ids.contains(&t.id), "{} is never produced", t.id);
    }
    for id in &ids {
        assert_eq!(TYPES.iter().filter(|t| t.id == *id).count(), 1, "{id}");
    }
}

#[test]
fn type_filter_picks_one_id_out_of_a_shared_kind() {
    let mut disk = Disk::new(64 * 1024);
    disk.put(0, &ole("WordDocument"));
    disk.put(8192, &ole("Workbook"));
    disk.put(16384, &tiff(b"II*\0", "NIKON"));
    let (files, _) =
        run(&disk, CarveOptions { types: vec!["xls".into(), "nef".into()], start: 0, end: None }, RangeSet::new());
    assert_eq!(files.iter().map(|f| (f.offset, f.type_id)).collect::<Vec<_>>(), vec![(8192, "xls"), (16384, "nef")]);
}

#[test]
fn random_data_is_not_a_file() {
    let mut x = 0x9E37_79B9_7F4A_7C15u64;
    let mut disk = Disk::new(4 * 1024 * 1024);
    for b in disk.data.iter_mut() {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        *b = x as u8;
    }
    let (files, _) = run(&disk, all(), RangeSet::new());
    assert!(files.is_empty(), "{files:?}");
}

#[test]
fn near_miss_headers_are_rejected() {
    let mut bad_tar = tar_header("a.txt", 1, b'0');
    bad_tar[0] ^= 1;
    let mut bad_regf = regf();
    bad_regf[0x1FC] ^= 1;
    let mut bad_ico = ico();
    bad_ico[18] += 16; // first image not right after the directory
    let mut bad_bmp = bmp();
    bad_bmp[14] = 41;
    let mut bad_ole = ole("WordDocument");
    bad_ole[0x1C] = 0xFF;
    let mut bad_xz = xz();
    bad_xz[8] ^= 1;
    let cases: Vec<(&str, Vec<u8>)> = vec![
        ("MZ without PE", b"MZ\x90\0\x03\0\0\0\x04\0\0\0\xFF\xFF\0\0".to_vec()),
        ("bmp dib size", bad_bmp),
        ("tiff odd ifd", b"II*\0\x07\0\0\0\0\0\0\0\0\0\0\0".to_vec()),
        ("ts with 4 packets", ts(188)[..188 * 4].to_vec()),
        ("id3 size not syncsafe", b"ID3\x03\0\0\0\0\x80\0\0\0\0\0\0\0".to_vec()),
        ("riff unknown form", b"RIFF\0\x10\0\0XXXXfmt ".to_vec()),
        ("tar checksum", bad_tar),
        ("font search range", b"\0\x01\0\0\0\x02\0\x10\0\x01\0\0\0\0\0\0".to_vec()),
        ("ico offset", bad_ico),
        ("ole byte order", bad_ole),
        ("xz flags crc", bad_xz),
        ("regf checksum", bad_regf),
        ("flac streaminfo length", b"fLaC\0\0\0\x21\0\0\0\0\0\0\0\0".to_vec()),
    ];
    for (what, b) in cases {
        assert_eq!(formats::identify(&b), None, "{what}");
    }
}
