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

Early. The imaging engine (`sf-core`) and its CLI (`sf-cli`) are working and
tested. Partition detection, file carving, and the GUI are not built yet.

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
    actually bad.
- `crates/sf-cli` — CLI wrapping the engine.

## Using the CLI

```powershell
# Clone a raw physical drive (run as Administrator). The mapfile lets you
# stop (Ctrl+C) and resume later without re-copying already-rescued data.
sf-cli image --source \\.\PhysicalDrive1 --dest E:\image.img --map E:\image.map

# Or image an existing file / test without touching real hardware:
sf-cli image --source C:\path\to\source.img --dest C:\path\to\clone.img --map C:\path\to\run.map

# Check progress on a mapfile at any time:
sf-cli status --map E:\image.map
```

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

- Linux raw device backend is unverified on real hardware.
- No `--reverse` (read backward) retry direction yet, unlike ddrescue.
- No sparse-file preallocation on the destination image.
- No partition table / filesystem detection yet (MBR/GPT, NTFS/FAT/exFAT/ext*).
- No file carving / deep recovery yet.
- No GUI yet (planned: Tauri, reusing `sf-core` directly — see the design
  mockup artifact from project planning).
