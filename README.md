# SectorForge

A cross-platform drive-imaging and recovery tool: clone a failing drive
sector-by-sector, isolate bad regions instead of giving up on them, then
recover partitions and files from the resulting image.

Built because DMDE works but is dated and confusing (dense, abbreviation-heavy
UI, no clear explanation of what SCSI/ATA/file access modes actually do).
SectorForge aims to combine the best ideas from DMDE, TestDisk/PhotoRec,
ddrescue/ddrescueview, R-Studio and UFS Explorer behind a much clearer UI (see
the design mockup in the project notes) — and to actually explain itself while
it runs.

## Status

Early. Working and tested: the imaging engine, partition detection, drive
listing, signature-based file carving, NTFS file listing/recovery (including
deleted files), and the CLI with a JSON mode. In progress: the GUI.

## Layout

- `crates/sf-core` — the engine: cross-platform raw block device access,
  a resumable ddrescue-style mapfile, and the imaging algorithm.
  - `device/windows.rs` — `\\.\PhysicalDriveN` via `FILE_FLAG_NO_BUFFERING`
    (direct hardware access, bypasses OS cache).
  - `device/linux.rs` — `/dev/sdX` via `O_DIRECT`. **Not yet tested on real
    Linux hardware** — written against documented ioctl numbers, needs
    validation on an actual box before trusting it on a real drive.
  - `device/file_backend.rs` — plain file backend ("Image file" mode): an
    existing disk image, or a stand-in for a device in tests/dev without
    needing admin rights or real hardware.
  - `mapfile.rs` — tracks which byte ranges are rescued / bad / not yet
    tried; saves in a ddrescue-compatible text format so an interrupted run
    resumes correctly and the map can be opened in `ddrescueview`.
  - `imager.rs` — two-pass algorithm: a fast forward sweep that skips past
    damage to maximize rescued data quickly, then a retry pass that
    binary-splits damaged regions down to the individual sectors that are
    actually bad. Checkpoints the map every 5 s, so killing it loses at most
    5 s of progress.
  - `partitions.rs` — MBR (including extended/logical), GPT (CRC-checked,
    falls back to the backup header), filesystems with no partition table,
    and a signature scan that finds NTFS / FAT12/16/32 / exFAT / ext2-4
    volumes when the table is damaged, or that it no longer lists.
  - `drives/` — lists physical drives (works without Administrator/root) and
    works out which physical disk any output path is on, so nothing is ever
    written onto the drive being rescued.
  - `reader.rs` — reads that never give up: unreadable bytes (bad sectors,
    or ranges the imaging map says were never rescued) come back as zeros
    *and are reported*, so results can say "partial" instead of lying.
  - `carve/` — PhotoRec-style recovery by file signature: JPEG, PNG, GIF,
    PDF, ZIP (and docx/xlsx/pptx), MP4/MOV, SQLite, 7z, RAR. Each format's
    real end is found from its own structure, not guessed.
  - `ntfs/` — reads the MFT directly: lists every file and folder including
    deleted ones, rebuilds paths (with sequence-number checks so a reused
    folder doesn't produce a wrong path), judges recoverability from the
    cluster bitmap and the imaging map, and extracts files with sanitized
    names.
- `crates/sf-cli` — CLI wrapping the engine. `--json` emits the NDJSON
  contract in [`docs/cli-contract.md`](docs/cli-contract.md), which is how the
  GUI drives it.
- `gui/` — Tauri desktop app; runs `sf-cli --json` as a child process.

## Using the CLI

```powershell
# Clone a raw physical drive (run as Administrator). The mapfile lets you
# stop (Ctrl+C) and resume later without re-copying already-rescued data.
sf-cli image --source \\.\PhysicalDrive1 --dest E:\image.img --map E:\image.map

# Or image an existing file / test without touching real hardware:
sf-cli image --source C:\path\to\source.img --dest C:\path\to\clone.img --map C:\path\to\run.map

# Check progress on a mapfile at any time:
sf-cli status --map E:\image.map

# Find partitions on the image (add --deep to also hunt for deleted or
# unlisted ones by filesystem signature — reads the whole image):
sf-cli scan --source E:\image.img
```

```powershell
# What drives are there? (works without Administrator)
sf-cli devices

# List files on the NTFS partition found by `scan` (start LBA 2048 here),
# including deleted ones; pass the imaging map so unrescued areas are
# reported honestly:
sf-cli files --source E:\image.img --partition-lba 2048 --map E:\image.map

# Copy files out by id (a folder id copies everything under it):
sf-cli extract --source E:\image.img --partition-lba 2048 --ids 4211,5120 --out F:\recovered

# No usable filesystem? Recover files by signature instead:
sf-cli carve --list-types
sf-cli carve --source E:\image.img --out F:\carved --types jpg,pdf,docx
```

Recovery commands refuse to write their output onto the drive being read in
device mode, and it's better still to run them against the cloned image
rather than the failing drive.

Add `--json` to any command for machine-readable output.

Any source path starting with `\\.\` (Windows) or `/dev/` (Linux) is treated
as a raw device (direct hardware access); anything else is opened as a plain
file.

## Testing

```powershell
cargo test
```

The engine's bad-sector/retry/resume logic is tested against an in-memory
fake block device with injected failures (`FakeBlockDevice`) — safe to run
anywhere, no real hardware involved. Carving is tested per format with
synthetic files embedded in a fake disk; the NTFS reader against a
hand-built volume (fragmented MFT, torn records, reused parents, hostile
names) *and* against a real Windows-formatted volume
(`crates/sf-core/tests/fixtures/ntfs-small.vhd.gz`, made by
`make-ntfs-fixture.ps1` next to it), checked file-by-file against SHA-256s
recorded when the files were written.

End-to-end contract check (builds synthetic disks, runs every command and
error path, validates the NDJSON and exit codes; `-Samples` refreshes
`docs/contract-samples/`):

```powershell
powershell -ExecutionPolicy Bypass -File scripts\contract-smoke.ps1
```

Linux raw-device check against real loop devices (512- and 4096-byte
sectors; needs sudo for `losetup`):

```bash
scripts/linux-loopdev-test.sh
```

## Known gaps / next steps

- Neither raw device backend has read real hardware yet. The Windows one has
  been checked only as far as refusing access without Administrator (drive
  *listing* and the same-drive check do work unelevated and were run against
  real drives). The Linux backend compiles and has a loop-device test
  (`scripts/linux-loopdev-test.sh`), but that test hasn't been run yet: no
  Linux machine or WSL was available.
- The same-drive check follows a volume down to its physical disks
  (Windows disk extents; Linux partitions, device-mapper and md). Storage
  Spaces on Windows reports its virtual disk rather than the member drives,
  so that case isn't caught; when the disk can't be determined at all the
  CLI refuses rather than guessing.
- NTFS-compressed and EFS-encrypted files are listed and flagged
  (`partial`, `complete:false` with a note) but not decoded; LZNT1
  decompression is next.
- `files`/`extract` support NTFS only (no FAT/exFAT/ext yet). They need a
  readable boot sector at the given LBA (the backup boot sector at the end
  of the volume isn't tried); a damaged `$MFT` record 0 falls back to the
  `$MFTMirr` copy.
- A deleted file whose clusters were reused is extracted with whatever now
  occupies them (and flagged); there's no attempt to tell old data from new.
- Carving finds files only at sector boundaries (as PhotoRec does by
  default), doesn't reassemble fragmented files, and a file whose end can't
  be found is cut at the next recognizable header or a per-type limit.
- MBR offsets assume 512-byte sectors (normal for MBR disks; native-4K drives
  use GPT, which handles any sector size).
- The signature scan skips a whole 4 MiB chunk if any read in it fails, so on
  a failing *device* it can miss a filesystem that starts in that chunk.
  Scanning the cloned image avoids this.
- Map checkpoints survive the process being killed, but not a power cut
  (the destination file isn't fsynced).
- No `--reverse` (read backward) retry direction yet, unlike ddrescue.
