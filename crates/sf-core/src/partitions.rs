//! Partition table parsing (MBR incl. extended/logical, GPT) and a
//! signature scan that finds filesystems when the table is damaged, missing,
//! or no longer lists a deleted partition.

use std::io;

use crate::block_device::BlockDevice;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FsKind {
    Ntfs,
    Fat12,
    Fat16,
    Fat32,
    ExFat,
    Ext,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Confidence {
    /// Listed in a valid table (or sitting at sector 0 with no table) and
    /// its boot sector matches a known filesystem.
    Confident,
    /// Found only by signature scan; nothing in any table points at it.
    Rebuilt,
    /// Listed in a table, but its boot sector isn't a filesystem we know.
    LowConfidence,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PartitionSource {
    Mbr,
    Gpt,
    SignatureScan,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TableKind {
    Mbr,
    Gpt,
    None,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PartitionInfo {
    pub start_bytes: u64,
    pub size_bytes: Option<u64>,
    pub filesystem: FsKind,
    pub confidence: Confidence,
    pub source: PartitionSource,
    pub label: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ScanResult {
    pub table_kind: TableKind,
    pub partitions: Vec<PartitionInfo>,
}

/// Bytes needed to recognize any supported filesystem from its start: boot
/// sector fields fit in 512, the ext superblock fields we read end at 1084.
pub const DETECT_WINDOW: usize = 1088;

const SIGNATURE_SCAN_CHUNK: u64 = 4 * 1024 * 1024;

/// Identifies the filesystem whose first byte is `buf[0]`. Tolerates short
/// buffers (just recognizes less).
pub fn detect_filesystem_at(buf: &[u8]) -> FsKind {
    if buf.len() >= 512 {
        let valid_bps = matches!(u16_le(buf, 11), 512 | 1024 | 2048 | 4096);
        if &buf[3..11] == b"NTFS    " && valid_bps {
            return FsKind::Ntfs;
        }
        if &buf[3..11] == b"EXFAT   " && (9..=12).contains(&buf[0x6C]) {
            return FsKind::ExFat;
        }
        if buf[510] == 0x55 && buf[511] == 0xAA && valid_bps && matches!(buf[0], 0xEB | 0xE9) {
            if &buf[82..90] == b"FAT32   " {
                return FsKind::Fat32;
            }
            if &buf[54..62] == b"FAT16   " {
                return FsKind::Fat16;
            }
            if &buf[54..62] == b"FAT12   " {
                return FsKind::Fat12;
            }
        }
    }
    if buf.len() >= 1084 && u16_le(buf, 1024 + 56) == 0xEF53 && u32_le(buf, 1024 + 24) <= 6 {
        return FsKind::Ext;
    }
    FsKind::Unknown
}

/// The filesystem's own idea of its size, read from its boot sector or
/// superblock. `None` if the fields are missing or implausible.
pub fn volume_size_bytes(kind: FsKind, buf: &[u8]) -> Option<u64> {
    let size = match kind {
        FsKind::Ntfs if buf.len() >= 0x30 => {
            // +1: the backup boot sector sits just past the counted sectors.
            (u64_le(buf, 0x28) + 1).checked_mul(u16_le(buf, 11) as u64)?
        }
        FsKind::Fat12 | FsKind::Fat16 | FsKind::Fat32 if buf.len() >= 36 => {
            let total16 = u16_le(buf, 19) as u64;
            let total = if total16 != 0 { total16 } else { u32_le(buf, 32) as u64 };
            total.checked_mul(u16_le(buf, 11) as u64)?
        }
        FsKind::ExFat if buf.len() >= 0x6D => {
            let shift = buf[0x6C] as u32;
            u64_le(buf, 0x48).checked_shl(shift)?
        }
        FsKind::Ext if buf.len() >= 1084 => {
            let blocks = u32_le(buf, 1024 + 4) as u64;
            let block_size = 1024u64 << u32_le(buf, 1024 + 24).min(6);
            blocks.checked_mul(block_size)?
        }
        _ => return None,
    };
    (size > 0).then_some(size)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MbrEntry {
    pub boot_flag: u8,
    pub part_type: u8,
    /// In 512-byte units, as MBR defines it.
    pub start_lba: u32,
    pub num_sectors: u32,
}

/// Parses the four primary entries of an MBR / EBR sector. `None` if the
/// 0x55AA signature is missing.
pub fn parse_mbr(sector: &[u8]) -> Option<[MbrEntry; 4]> {
    if sector.len() < 512 || sector[510] != 0x55 || sector[511] != 0xAA {
        return None;
    }
    let entry = |i: usize| {
        let off = 446 + i * 16;
        MbrEntry {
            boot_flag: sector[off],
            part_type: sector[off + 4],
            start_lba: u32_le(sector, off + 8),
            num_sectors: u32_le(sector, off + 12),
        }
    };
    Some([entry(0), entry(1), entry(2), entry(3)])
}

fn is_extended(part_type: u8) -> bool {
    matches!(part_type, 0x05 | 0x0F | 0x85)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GptHeader {
    pub partition_entry_lba: u64,
    pub num_entries: u32,
    pub entry_size: u32,
}

/// Parses and CRC-checks a GPT header. `None` if the signature or header
/// checksum is wrong, or the entry geometry is implausible.
pub fn parse_gpt_header(buf: &[u8]) -> Option<GptHeader> {
    if buf.len() < 92 || &buf[0..8] != b"EFI PART" {
        return None;
    }
    let header_size = u32_le(buf, 12) as usize;
    if !(92..=buf.len()).contains(&header_size) {
        return None;
    }
    let mut copy = buf[..header_size].to_vec();
    copy[16..20].fill(0);
    if crc32(&copy) != u32_le(buf, 16) {
        return None;
    }
    let num_entries = u32_le(buf, 80);
    let entry_size = u32_le(buf, 84);
    if num_entries == 0 || num_entries > 1024 || !(128..=4096).contains(&entry_size) || entry_size % 8 != 0 {
        return None;
    }
    Some(GptHeader {
        partition_entry_lba: u64_le(buf, 72),
        num_entries,
        entry_size,
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GptEntry {
    pub type_guid: [u8; 16],
    pub start_lba: u64,
    pub end_lba: u64,
    pub name: String,
}

/// `None` for an unused (all-zero type GUID) entry.
pub fn parse_gpt_entry(buf: &[u8]) -> Option<GptEntry> {
    if buf.len() < 128 {
        return None;
    }
    let type_guid: [u8; 16] = buf[0..16].try_into().unwrap();
    if type_guid == [0u8; 16] {
        return None;
    }
    let units: Vec<u16> = buf[56..128]
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .take_while(|&u| u != 0)
        .collect();
    Some(GptEntry {
        type_guid,
        start_lba: u64_le(buf, 32),
        end_lba: u64_le(buf, 40),
        name: String::from_utf16_lossy(&units),
    })
}

/// Reads the partition table, falling back to (or, with `deep`, adding) a
/// full-device signature scan. `on_progress(bytes_scanned, bytes_total)` is
/// only called during a signature scan.
pub fn scan<D: BlockDevice>(
    device: &mut D,
    deep: bool,
    on_progress: impl FnMut(u64, u64),
) -> io::Result<ScanResult> {
    let sector0 = read_up_to(device, 0, DETECT_WINDOW)?;

    // A filesystem directly at sector 0 means no partition table at all
    // (e.g. a USB stick formatted "superfloppy" style). Its boot sector also
    // ends in 0x55AA, so this must be checked before treating it as an MBR.
    let fs0 = detect_filesystem_at(&sector0);
    let (table_kind, mut partitions) = if fs0 != FsKind::Unknown {
        let info = PartitionInfo {
            start_bytes: 0,
            size_bytes: volume_size_bytes(fs0, &sector0),
            filesystem: fs0,
            confidence: Confidence::Confident,
            source: PartitionSource::SignatureScan,
            label: None,
        };
        (TableKind::None, vec![info])
    } else if let Some(entries) = parse_mbr(&sector0) {
        if entries.iter().any(|e| e.part_type == 0xEE) {
            match read_gpt(device)? {
                Some(parts) => (TableKind::Gpt, parts),
                None => (TableKind::None, Vec::new()),
            }
        } else {
            let parts = read_mbr_partitions(device, &entries)?;
            let kind = if parts.is_empty() { TableKind::None } else { TableKind::Mbr };
            (kind, parts)
        }
    } else {
        (TableKind::None, Vec::new())
    };

    if deep || partitions.is_empty() {
        for found in signature_scan(device, on_progress)? {
            if !partitions.iter().any(|p| p.start_bytes == found.start_bytes) {
                partitions.push(found);
            }
        }
        partitions.sort_by_key(|p| p.start_bytes);
    }

    Ok(ScanResult { table_kind, partitions })
}

fn read_mbr_partitions<D: BlockDevice>(device: &mut D, primary: &[MbrEntry; 4]) -> io::Result<Vec<PartitionInfo>> {
    let mut out = Vec::new();
    for e in primary.iter().filter(|e| e.part_type != 0 && e.num_sectors != 0) {
        if is_extended(e.part_type) {
            read_logical_partitions(device, e.start_lba as u64, &mut out)?;
        } else {
            out.push(table_partition(device, e.start_lba as u64 * 512, Some(e.num_sectors as u64 * 512), PartitionSource::Mbr, None)?);
        }
    }
    Ok(out)
}

/// Walks the EBR chain inside an extended partition. Each EBR's first entry
/// is a logical partition (relative to that EBR); its second points to the
/// next EBR (relative to the start of the whole extended partition).
fn read_logical_partitions<D: BlockDevice>(device: &mut D, extended_start: u64, out: &mut Vec<PartitionInfo>) -> io::Result<()> {
    let mut ebr_lba = extended_start;
    let mut seen = std::collections::HashSet::new();
    while seen.insert(ebr_lba) && seen.len() <= 256 {
        let ebr = read_up_to(device, ebr_lba * 512, 512)?;
        let Some(entries) = parse_mbr(&ebr) else { break };
        let logical = entries[0];
        if logical.part_type != 0 && logical.num_sectors != 0 {
            let start = (ebr_lba + logical.start_lba as u64) * 512;
            out.push(table_partition(device, start, Some(logical.num_sectors as u64 * 512), PartitionSource::Mbr, None)?);
        }
        let next = entries[1];
        if next.part_type == 0 || next.start_lba == 0 {
            break;
        }
        ebr_lba = extended_start + next.start_lba as u64;
    }
    Ok(())
}

fn read_gpt<D: BlockDevice>(device: &mut D) -> io::Result<Option<Vec<PartitionInfo>>> {
    let sector = device.sector_size();
    let primary = read_up_to(device, sector, sector as usize)?;
    let header = match parse_gpt_header(&primary) {
        Some(h) => h,
        None => {
            // Primary header damaged — the backup lives in the last sector.
            let last = device.size_bytes().saturating_sub(sector);
            let backup = read_up_to(device, last, sector as usize)?;
            match parse_gpt_header(&backup) {
                Some(h) => h,
                None => return Ok(None),
            }
        }
    };

    let array_len = header.num_entries as usize * header.entry_size as usize;
    let array = read_up_to(device, header.partition_entry_lba * sector, array_len)?;
    let mut out = Vec::new();
    for raw in array.chunks_exact(header.entry_size as usize) {
        let Some(entry) = parse_gpt_entry(raw) else { continue };
        if entry.end_lba < entry.start_lba {
            continue;
        }
        let size = (entry.end_lba - entry.start_lba + 1) * sector;
        let label = (!entry.name.is_empty()).then_some(entry.name);
        out.push(table_partition(device, entry.start_lba * sector, Some(size), PartitionSource::Gpt, label)?);
    }
    Ok(Some(out))
}

fn table_partition<D: BlockDevice>(
    device: &mut D,
    start_bytes: u64,
    size_bytes: Option<u64>,
    source: PartitionSource,
    label: Option<String>,
) -> io::Result<PartitionInfo> {
    // A boot sector we can't read (bad sector, or a table entry pointing past
    // the end of the disk) is "unknown", not a reason to abort the scan.
    let boot = read_up_to(device, start_bytes, DETECT_WINDOW).unwrap_or_default();
    let filesystem = detect_filesystem_at(&boot);
    let confidence = if filesystem == FsKind::Unknown {
        Confidence::LowConfidence
    } else {
        Confidence::Confident
    };
    Ok(PartitionInfo {
        start_bytes,
        size_bytes,
        filesystem,
        confidence,
        source,
        label,
    })
}

/// Reads the whole device in large chunks and checks every sector boundary
/// for a filesystem start. When one is found, jumps past the volume using
/// its own size field so its contents (and its backup boot sector) aren't
/// mistaken for more partitions. Unreadable chunks are skipped, not fatal.
pub fn signature_scan<D: BlockDevice>(
    device: &mut D,
    mut on_progress: impl FnMut(u64, u64),
) -> io::Result<Vec<PartitionInfo>> {
    let sector = device.sector_size();
    let total = device.size_bytes();
    let mut out = Vec::new();
    let mut pos = 0u64;
    let mut skip_until = 0u64;

    while pos < total {
        let chunk = SIGNATURE_SCAN_CHUNK.min(total - pos);
        // Over-read so a filesystem starting near the end of this chunk can
        // still be recognized in full.
        let want = (chunk + DETECT_WINDOW as u64).min(total - pos) as usize;
        if let Ok(buf) = read_up_to(device, pos, want) {
            let mut local = 0u64;
            while local < chunk {
                let abs = pos + local;
                if abs >= skip_until {
                    let window = &buf[local as usize..buf.len().min(local as usize + DETECT_WINDOW)];
                    let fs = detect_filesystem_at(window);
                    if fs != FsKind::Unknown {
                        let size = volume_size_bytes(fs, window).filter(|&s| s <= total - abs);
                        out.push(PartitionInfo {
                            start_bytes: abs,
                            size_bytes: size,
                            filesystem: fs,
                            confidence: Confidence::Rebuilt,
                            source: PartitionSource::SignatureScan,
                            label: None,
                        });
                        skip_until = abs + size.unwrap_or(sector).max(sector);
                    }
                }
                local += sector;
            }
        }
        pos += chunk;
        on_progress(pos, total);
    }
    Ok(out)
}

/// Reads `want_len` bytes at an arbitrary `byte_offset` (clipped to the end
/// of the device), issuing a sector-aligned read underneath as
/// `BlockDevice` requires. Assumes the device size is a whole number of
/// sectors, which holds for any real block device.
pub fn read_up_to<D: BlockDevice>(device: &mut D, byte_offset: u64, want_len: usize) -> io::Result<Vec<u8>> {
    let total = device.size_bytes();
    if byte_offset >= total || want_len == 0 {
        return Ok(Vec::new());
    }
    let sector = device.sector_size();
    let aligned_start = byte_offset - byte_offset % sector;
    let end = (byte_offset + want_len as u64).min(total);
    let aligned_end = end.div_ceil(sector) * sector;
    let mut buf = vec![0u8; (aligned_end.min(total) - aligned_start) as usize];
    device.read_at(aligned_start, &mut buf)?;
    let from = (byte_offset - aligned_start) as usize;
    let to = from + (end - byte_offset) as usize;
    Ok(buf[from..to].to_vec())
}

fn u16_le(b: &[u8], off: usize) -> u16 {
    u16::from_le_bytes([b[off], b[off + 1]])
}

fn u32_le(b: &[u8], off: usize) -> u32 {
    u32::from_le_bytes(b[off..off + 4].try_into().unwrap())
}

fn u64_le(b: &[u8], off: usize) -> u64 {
    u64::from_le_bytes(b[off..off + 8].try_into().unwrap())
}

/// CRC-32 (IEEE 802.3), as used by GPT headers and entry arrays.
pub fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &byte in data {
        crc ^= byte as u32;
        for _ in 0..8 {
            crc = if crc & 1 != 0 { (crc >> 1) ^ 0xEDB8_8320 } else { crc >> 1 };
        }
    }
    !crc
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block_device::FakeBlockDevice;

    const MIB: usize = 1024 * 1024;

    fn ntfs_boot(total_sectors: u64) -> Vec<u8> {
        let mut b = vec![0u8; 512];
        b[0..3].copy_from_slice(&[0xEB, 0x52, 0x90]);
        b[3..11].copy_from_slice(b"NTFS    ");
        b[11..13].copy_from_slice(&512u16.to_le_bytes());
        b[0x28..0x30].copy_from_slice(&total_sectors.to_le_bytes());
        b[510] = 0x55;
        b[511] = 0xAA;
        b
    }

    fn fat32_boot(total_sectors: u32) -> Vec<u8> {
        let mut b = vec![0u8; 512];
        b[0..3].copy_from_slice(&[0xEB, 0x58, 0x90]);
        b[3..11].copy_from_slice(b"MSDOS5.0");
        b[11..13].copy_from_slice(&512u16.to_le_bytes());
        b[32..36].copy_from_slice(&total_sectors.to_le_bytes());
        b[82..90].copy_from_slice(b"FAT32   ");
        b[510] = 0x55;
        b[511] = 0xAA;
        b
    }

    fn ext_start(blocks: u32, log_block_size: u32) -> Vec<u8> {
        let mut b = vec![0u8; 2048];
        b[1024 + 4..1024 + 8].copy_from_slice(&blocks.to_le_bytes());
        b[1024 + 24..1024 + 28].copy_from_slice(&log_block_size.to_le_bytes());
        b[1024 + 56..1024 + 58].copy_from_slice(&0xEF53u16.to_le_bytes());
        b
    }

    fn mbr_entry(disk: &mut [u8], sector_offset: usize, slot: usize, part_type: u8, start: u32, len: u32) {
        let off = sector_offset + 446 + slot * 16;
        disk[off + 4] = part_type;
        disk[off + 8..off + 12].copy_from_slice(&start.to_le_bytes());
        disk[off + 12..off + 16].copy_from_slice(&len.to_le_bytes());
        disk[sector_offset + 510] = 0x55;
        disk[sector_offset + 511] = 0xAA;
    }

    fn put(disk: &mut [u8], at: usize, bytes: &[u8]) {
        disk[at..at + bytes.len()].copy_from_slice(bytes);
    }

    #[test]
    fn crc32_matches_known_vector() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    }

    #[test]
    fn detects_each_filesystem_and_its_size() {
        let ntfs = ntfs_boot(999);
        assert_eq!(detect_filesystem_at(&ntfs), FsKind::Ntfs);
        assert_eq!(volume_size_bytes(FsKind::Ntfs, &ntfs), Some(1000 * 512));

        let fat = fat32_boot(4096);
        assert_eq!(detect_filesystem_at(&fat), FsKind::Fat32);
        assert_eq!(volume_size_bytes(FsKind::Fat32, &fat), Some(4096 * 512));

        let ext = ext_start(100, 2); // 4 KiB blocks
        assert_eq!(detect_filesystem_at(&ext), FsKind::Ext);
        assert_eq!(volume_size_bytes(FsKind::Ext, &ext), Some(100 * 4096));

        assert_eq!(detect_filesystem_at(&[0u8; 2048]), FsKind::Unknown);
    }

    #[test]
    fn rejects_ntfs_string_with_nonsense_sector_size() {
        let mut b = ntfs_boot(10);
        b[11..13].copy_from_slice(&333u16.to_le_bytes());
        assert_eq!(detect_filesystem_at(&b), FsKind::Unknown);
    }

    #[test]
    fn reads_mbr_primary_and_logical_partitions() {
        let mut disk = vec![0u8; 4 * MIB];
        // Primary NTFS at LBA 64, extended partition at LBA 2048 holding one
        // logical FAT32 at EBR+64.
        mbr_entry(&mut disk, 0, 0, 0x07, 64, 1000);
        mbr_entry(&mut disk, 0, 1, 0x0F, 2048, 4000);
        put(&mut disk, 64 * 512, &ntfs_boot(999));
        mbr_entry(&mut disk, 2048 * 512, 0, 0x0C, 64, 2000);
        put(&mut disk, (2048 + 64) * 512, &fat32_boot(2000));

        let mut dev = FakeBlockDevice::new(disk, 512);
        let result = scan(&mut dev, false, |_, _| {}).unwrap();

        assert_eq!(result.table_kind, TableKind::Mbr);
        assert_eq!(result.partitions.len(), 2);
        assert_eq!(result.partitions[0].start_bytes, 64 * 512);
        assert_eq!(result.partitions[0].filesystem, FsKind::Ntfs);
        assert_eq!(result.partitions[0].confidence, Confidence::Confident);
        assert_eq!(result.partitions[1].start_bytes, (2048 + 64) * 512);
        assert_eq!(result.partitions[1].filesystem, FsKind::Fat32);
        assert_eq!(result.partitions[1].size_bytes, Some(2000 * 512));
    }

    #[test]
    fn table_entry_without_a_known_filesystem_is_low_confidence() {
        let mut disk = vec![0u8; MIB];
        mbr_entry(&mut disk, 0, 0, 0x07, 64, 100);
        let mut dev = FakeBlockDevice::new(disk, 512);
        let result = scan(&mut dev, false, |_, _| {}).unwrap();
        assert_eq!(result.partitions[0].confidence, Confidence::LowConfidence);
        assert_eq!(result.partitions[0].filesystem, FsKind::Unknown);
    }

    fn build_gpt_disk(corrupt_primary: bool) -> Vec<u8> {
        let size = 4 * MIB;
        let last_lba = (size / 512 - 1) as u64;
        let mut disk = vec![0u8; size];
        mbr_entry(&mut disk, 0, 0, 0xEE, 1, u32::MAX);

        let mut entries = vec![0u8; 128 * 128];
        entries[0..16].copy_from_slice(&[0xA2; 16]); // any non-zero type GUID
        entries[32..40].copy_from_slice(&2048u64.to_le_bytes());
        entries[40..48].copy_from_slice(&4095u64.to_le_bytes());
        for (i, c) in "Data".encode_utf16().enumerate() {
            entries[56 + i * 2..58 + i * 2].copy_from_slice(&c.to_le_bytes());
        }

        let header = |entry_lba: u64| {
            let mut h = vec![0u8; 92];
            h[0..8].copy_from_slice(b"EFI PART");
            h[12..16].copy_from_slice(&92u32.to_le_bytes());
            h[72..80].copy_from_slice(&entry_lba.to_le_bytes());
            h[80..84].copy_from_slice(&128u32.to_le_bytes());
            h[84..88].copy_from_slice(&128u32.to_le_bytes());
            h[88..92].copy_from_slice(&crc32(&entries).to_le_bytes());
            let crc = crc32(&h);
            h[16..20].copy_from_slice(&crc.to_le_bytes());
            h
        };

        put(&mut disk, 2 * 512, &entries);
        put(&mut disk, 512, &header(2));
        let backup_entries_lba = last_lba - 32;
        put(&mut disk, backup_entries_lba as usize * 512, &entries);
        put(&mut disk, last_lba as usize * 512, &header(backup_entries_lba));
        put(&mut disk, 2048 * 512, &ntfs_boot(2047));

        if corrupt_primary {
            disk[512 + 30] ^= 0xFF; // breaks the header CRC
        }
        disk
    }

    #[test]
    fn reads_gpt_with_label() {
        let mut dev = FakeBlockDevice::new(build_gpt_disk(false), 512);
        let result = scan(&mut dev, false, |_, _| {}).unwrap();
        assert_eq!(result.table_kind, TableKind::Gpt);
        assert_eq!(result.partitions.len(), 1);
        let p = &result.partitions[0];
        assert_eq!(p.start_bytes, 2048 * 512);
        assert_eq!(p.size_bytes, Some(2048 * 512));
        assert_eq!(p.filesystem, FsKind::Ntfs);
        assert_eq!(p.label.as_deref(), Some("Data"));
    }

    #[test]
    fn falls_back_to_backup_gpt_header_when_primary_is_corrupt() {
        let mut dev = FakeBlockDevice::new(build_gpt_disk(true), 512);
        let result = scan(&mut dev, false, |_, _| {}).unwrap();
        assert_eq!(result.table_kind, TableKind::Gpt);
        assert_eq!(result.partitions[0].start_bytes, 2048 * 512);
    }

    #[test]
    fn superfloppy_filesystem_at_sector_zero_is_not_mistaken_for_an_mbr() {
        let mut disk = vec![0u8; MIB];
        put(&mut disk, 0, &fat32_boot((MIB / 512) as u32));
        let mut dev = FakeBlockDevice::new(disk, 512);
        let result = scan(&mut dev, false, |_, _| {}).unwrap();
        assert_eq!(result.table_kind, TableKind::None);
        assert_eq!(result.partitions.len(), 1);
        assert_eq!(result.partitions[0].filesystem, FsKind::Fat32);
        assert_eq!(result.partitions[0].start_bytes, 0);
    }

    #[test]
    fn wiped_table_falls_back_to_signature_scan() {
        let mut disk = vec![0u8; 12 * MIB];
        // Two volumes, no table. The NTFS one also has its backup boot
        // sector at its last sector, which must not be reported separately.
        put(&mut disk, MIB, &ntfs_boot(2047)); // 1 MiB volume
        put(&mut disk, MIB + 2047 * 512, &ntfs_boot(2047));
        // Last sector of a 4 MiB read chunk: its superblock is in the next
        // chunk, so this is only found if chunks over-read.
        let ext_at = 8 * MIB - 512;
        put(&mut disk, ext_at, &ext_start(512, 0));

        let mut dev = FakeBlockDevice::new(disk, 512);
        let mut last_progress = (0, 0);
        let result = scan(&mut dev, false, |done, total| last_progress = (done, total)).unwrap();

        assert_eq!(result.table_kind, TableKind::None);
        let found: Vec<(u64, FsKind)> = result.partitions.iter().map(|p| (p.start_bytes, p.filesystem)).collect();
        assert_eq!(found, vec![(MIB as u64, FsKind::Ntfs), (ext_at as u64, FsKind::Ext)]);
        assert!(result.partitions.iter().all(|p| p.confidence == Confidence::Rebuilt));
        assert_eq!(last_progress, (12 * MIB as u64, 12 * MIB as u64));
    }

    #[test]
    fn deep_scan_finds_a_deleted_partition_alongside_the_table() {
        let mut disk = vec![0u8; 8 * MIB];
        mbr_entry(&mut disk, 0, 0, 0x07, 64, 2048);
        put(&mut disk, 64 * 512, &ntfs_boot(2047));
        put(&mut disk, 5 * MIB, &fat32_boot(2048)); // not in the table

        let mut dev = FakeBlockDevice::new(disk.clone(), 512);
        assert_eq!(scan(&mut dev, false, |_, _| {}).unwrap().partitions.len(), 1);

        let mut dev = FakeBlockDevice::new(disk, 512);
        let result = scan(&mut dev, true, |_, _| {}).unwrap();
        assert_eq!(result.table_kind, TableKind::Mbr);
        assert_eq!(result.partitions.len(), 2);
        assert_eq!(result.partitions[1].start_bytes, 5 * MIB as u64);
        assert_eq!(result.partitions[1].source, PartitionSource::SignatureScan);
    }

    #[test]
    fn signature_scan_survives_unreadable_regions() {
        let mut disk = vec![0u8; 12 * MIB];
        put(&mut disk, 9 * MIB, &fat32_boot(2048));
        let mut dev = FakeBlockDevice::new(disk, 512);
        dev.mark_bad(MIB as u64, MIB as u64 + 512);
        let found = signature_scan(&mut dev, |_, _| {}).unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].start_bytes, 9 * MIB as u64);
    }

    #[test]
    fn read_up_to_handles_unaligned_ranges_and_the_device_end() {
        let data: Vec<u8> = (0..2048u32).map(|i| (i % 256) as u8).collect();
        let mut dev = FakeBlockDevice::new(data.clone(), 512);
        assert_eq!(read_up_to(&mut dev, 700, 100).unwrap(), data[700..800]);
        assert_eq!(read_up_to(&mut dev, 2000, 500).unwrap(), data[2000..2048]);
        assert!(read_up_to(&mut dev, 4096, 10).unwrap().is_empty());
    }
}
