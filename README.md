# SectorForge

[![CI](https://github.com/terraseraph/sectorforge/actions/workflows/ci.yml/badge.svg)](https://github.com/terraseraph/sectorforge/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/terraseraph/sectorforge)](https://github.com/terraseraph/sectorforge/releases/latest)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

Rescue data from a failing hard drive. SectorForge clones the drive
sector by sector, working around bad areas instead of giving up on them,
then finds the partitions and recovers your files from the copy, including
deleted ones.

It brings together ideas from ddrescue, TestDisk/PhotoRec, DMDE and R-Studio
behind an interface that explains what it is doing and why. It runs on
Windows and Linux as a portable app: no installer.

> **If a drive is failing, stop using it.** Every hour it runs can make
> things worse. Don't install anything onto it, and don't run repair tools
> (chkdsk, fsck) on it. Copy it first with SectorForge, then recover from the
> copy.
>
> A healthy drive (for example, one with accidentally deleted files) can be
> recovered from directly, live: pick it as the source on the Recover screen,
> or pass `\\.\PhysicalDriveN` / `/dev/sdX` to `scan`, `files`, `extract` and
> `carve`. Save recovered files to a different drive.

## Download

Get the latest from the **[Releases page](https://github.com/terraseraph/sectorforge/releases/latest)**.

| Platform | File | What it is |
|---|---|---|
| Windows 10/11 (x64) | `SectorForge-…-windows-x64.exe` | The app: one portable exe. Put it anywhere and run it. |
| Linux (x86_64) | `SectorForge-…-x86_64.AppImage` | The app. `chmod +x`, then run it. |
| Windows | `sf-cli-…-windows-x64.zip` | Command-line tool only. |
| Linux | `sf-cli-…-linux-x86_64-static.tar.gz` | Command-line tool only, fully static: runs on any distro. |

`SHA256SUMS.txt` lists checksums for every file.

**Windows notes**
- The exe isn't code-signed yet, so SmartScreen may warn the first time.
  Choose *More info → Run anyway*.
- The recovery engine is built into the exe, so there's nothing else to keep
  next to it.
- Reading a physical drive needs Administrator rights. Pick a drive and the
  app offers *Restart as Administrator*.
- Uses Windows' built-in WebView2, which is preinstalled on Windows 11 and
  current Windows 10.

**Linux notes**
- The AppImage needs FUSE 2. On Ubuntu 22.04+ install it with
  `sudo apt install libfuse2`.
- Reading a physical drive needs root. SectorForge asks for your password
  through `pkexec` (polkit) when a job needs it.
- Needs glibc 2.35 or newer (Ubuntu 22.04, Debian 12, Fedora 36, or later).
  On anything older, use the static command-line tool.

## How to recover a drive

1. **Image.** Pick the failing drive, choose where to save the copy (on a
   *different*, healthy drive), and start.
   - The first pass copies everything that reads cleanly and skips damaged
     areas, so the most data comes off before the drive gets any worse.
   - The second pass goes back and retries the damaged areas sector by
     sector.
   - The **drive health map** shows what's been rescued, what's being
     retried, and what's bad.
   - You can pause and resume at any time. The *map file* records what's
     been copied, so keep it with the image.
2. **Partitions.** Point it at the image. It reads the partition table, and
   with *Deep scan* it also searches the whole image for filesystems that
   the table no longer lists (damaged or deleted partitions).
3. **Recover.**
   - **Files** (NTFS, FAT, exFAT, ext) lists every file and folder with its real name and
     path, including deleted ones, and tells you which can be fully
     recovered. Tick what you want and extract it.
   - **Carve** finds files by their contents (photos, documents, archives,
     videos) when there's no usable filesystem at all. Files come back
     without their original names.

SectorForge refuses to write anything onto the drive it's reading from, and
it only ever *reads* the source drive.

## Features

- **Imaging:** two-pass, ddrescue-style cloning with bad-sector isolation,
  pause/resume, and a progress map saved every 5 seconds. The map uses
  ddrescue's format, so `ddrescueview` can open it too.
- **Direct hardware access:** reads the raw drive, bypassing the OS cache
  (`FILE_FLAG_NO_BUFFERING` on Windows, `O_DIRECT` on Linux), or works on an
  image file.
- **Partitions:**
  - MBR, including extended/logical partitions.
  - GPT, checksum-verified, with fallback to the backup header.
  - Disks with no partition table.
  - Signature scan for NTFS, FAT12/16/32, exFAT and ext2/3/4.
- **NTFS recovery:**
  - Reads the MFT directly: deleted files, fragmented files, sparse files,
    and hard links stored in attribute lists.
  - Rebuilds full paths.
  - Checks whether a deleted file's space has since been reused.
- **FAT12/16/32 and exFAT recovery:**
  - Long file names, deleted files and deleted folders.
  - A bad sector in the first FAT is read from the backup copy.
  - Deleted FAT files are read as one piece from their first cluster (FAT
    erases the map of where the pieces were); reused clusters are flagged.
- **ext2/3/4 recovery:**
  - Live files through extent trees or ext2 block maps.
  - Deleted names left in directory slack; recoverable only when the freed
    inode still says where the data is (ext3/ext4 usually erase that).
- **Carving:**
  - About 95 file types (`sf-cli carve --list-types`): photos and camera raws
    (JPEG, PNG, HEIC, TIFF, CR2/CR3, NEF, ARW, DNG…), documents (PDF, Office
    old and new, OpenDocument, RTF, EPUB…), audio, video, archives,
    executables, fonts, mail stores (PST/OST), registry hives and event logs.
  - Each file's end is found from its own structure rather than guessed.
- **Honest results:**
  - Pass the imaging map to recovery, and any file overlapping a sector that
    was never rescued is marked *partial* instead of silently containing
    blanks.
  - Recovered names are sanitized, so nothing can be written outside the
    output folder.

## Command line

Everything the app does is available from `sf-cli`. Add `--json` to any
command for machine-readable output (see the [JSON contract](docs/cli-contract.md)).

```powershell
sf-cli devices                                   # list drives (no admin needed)

# Clone a drive (as Administrator / root). Re-run the same command to resume.
sf-cli image --source \\.\PhysicalDrive1 --dest E:\rescue\disk.img --map E:\rescue\disk.map
sf-cli status --map E:\rescue\disk.map           # progress at any time

sf-cli scan --source E:\rescue\disk.img          # partitions (add --deep to hunt for lost ones)

# NTFS/FAT/exFAT/ext: list files incl. deleted (partition start LBA comes from scan), then extract by id
sf-cli files   --source E:\rescue\disk.img --partition-lba 2048 --map E:\rescue\disk.map
sf-cli extract --source E:\rescue\disk.img --partition-lba 2048 --ids 4211,5120 --out F:\recovered

# No filesystem? Recover by file signature:
sf-cli carve --list-types
sf-cli carve --source E:\rescue\disk.img --out F:\carved --types jpg,pdf,docx
```

On Linux, drives are `/dev/sdX` or `/dev/nvme0n1`. Any source starting with
`\\.\` or `/dev/` is read as a physical drive; anything else as an image file.

## Status and limitations

SectorForge is young. The engine is well tested against simulated failing
drives, a real Windows-formatted NTFS volume (every recovered file is checked
byte for byte against the original), and real files made by standard tools.
But:

- **Reading a physical drive end to end hasn't been tested yet**, on either
  platform. Listing drives and the safety checks have run on real hardware;
  imaging a real drive is next. On Linux, raw-device reads are tested in CI
  against loop devices, not real disks.
- NTFS-compressed and encrypted files are listed and flagged, but not
  decoded yet.
- Deleted files on FAT are assumed to be stored in one piece; a fragmented
  deleted file comes back with other data in its later parts. On ext3/ext4,
  deleted files usually can't be located by the filesystem at all: carve
  instead.
- FAT timestamps are local time with no zone; they're shown as if UTC.
- ext: symlinks, devices and inline data larger than 60 bytes aren't
  recovered; fscrypt-encrypted files are copied still encrypted.
- Carving doesn't reassemble fragmented files.
- If a deleted file's space was reused, what's recovered is whatever is
  there now (it's flagged, not hidden).
- On Windows Storage Spaces, the "don't write to the source drive" check sees
  the virtual disk rather than its member drives.
- A power cut during imaging can lose recent progress. Killing the program
  can't: at most the last 5 seconds are redone.

Please report problems in the [issue tracker](https://github.com/terraseraph/sectorforge/issues).
Back up anything you recover, and if the data is irreplaceable and
the drive is clicking or not spinning up, consider a professional recovery
lab before running *any* software on it.

## Building from source

Requirements: Rust (stable) and Node.js 22. On Linux you also need the Tauri
system libraries listed in [`gui/README.md`](gui/README.md).

```bash
cargo build --release -p sf-cli          # the engine + CLI
cargo test --workspace                   # engine tests

cd gui
npm ci
npm test && npm run build                # GUI frontend
npx tauri dev                            # run the app (finds ../target/release/sf-cli)
```

Release packages are built by [GitHub Actions](.github/workflows/release.yml)
when a `v*` tag is pushed. The steps are documented in [`gui/README.md`](gui/README.md).

### Layout

- `crates/sf-core`: the engine.
  - Raw device access, the resumable mapfile and the imaging algorithm.
  - Partition detection, drive listing and the same-drive check.
  - NTFS, FAT/exFAT and ext readers, and carving.
- `crates/sf-cli`: the command-line tool, a thin layer over the engine.
- `gui/`: Tauri 2 desktop app. It runs `sf-cli --json` as a separate process
  and never links the engine, so only `sf-cli` needs elevated rights.
- `docs/cli-contract.md`: the JSON protocol between them.
  `docs/contract-samples/` holds real output.
- `scripts/`: the end-to-end contract check (`contract-smoke.ps1`) and the
  Linux loop-device test.

## Acknowledgements

Inspired by [GNU ddrescue](https://www.gnu.org/software/ddrescue/),
[TestDisk & PhotoRec](https://www.cgsecurity.org/), DMDE, R-Studio and UFS
Explorer. SectorForge is an independent project, not affiliated with any of
them.

## License

[MIT](LICENSE). Provided as is, without warranty. Data recovery carries
risk; read the advice above before running anything on a failing drive.
