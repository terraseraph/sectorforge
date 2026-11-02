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

Early. Working and tested: the imaging engine, partition detection, and the
CLI with a JSON mode. In progress: the GUI. Not started: file carving.

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
anywhere, no real hardware involved. A real end-to-end smoke test (CLI
against a file-backed fake disk, verified byte-for-byte with a hash compare,
plus a resume check) has been run manually; it isn't automated yet.

## Known gaps / next steps

- Neither raw device backend has read real hardware yet. The Windows one has
  been checked only as far as refusing access without Administrator. The Linux
  one hasn't been run at all.
- MBR offsets assume 512-byte sectors (normal for MBR disks; native-4K drives
  use GPT, which handles any sector size).
- The signature scan skips a whole 4 MiB chunk if any read in it fails, so on
  a failing *device* it can miss a filesystem that starts in that chunk.
  Scanning the cloned image avoids this.
- Map checkpoints survive the process being killed, but not a power cut
  (the destination file isn't fsynced).
- No `--reverse` (read backward) retry direction yet, unlike ddrescue.
- No file carving / file-level recovery yet.
