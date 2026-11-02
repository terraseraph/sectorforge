# sf-cli JSON contract (v2)

v2 adds `version`, `devices`, `carve`, `files`, `extract`, and two new
`image` refusals. Everything in v1 is unchanged.

The GUI never links `sf-core` directly. It runs `sf-cli` as a child process
with `--json` and reads its stdout. This keeps privileged raw-disk access in a
separate process (only `sf-cli` needs Administrator/root), and lets the GUI be
replaced without touching the engine.

## Rules

- With `--json`, stdout carries **only** newline-delimited JSON (NDJSON): one
  complete JSON object per line, UTF-8, `\n` terminated. Nothing else is
  printed to stdout. Human-readable text, if any, goes to stderr.
- Every object has an `"event"` string field.
- All byte offsets and sizes are JSON integers, in **bytes**, except fields
  explicitly named `*_lba` / `*_sectors`, which are in **512-byte units**.
- Unknown fields must be ignored by the GUI (the CLI may add fields).
- Exit code: `0` success, `1` runtime error (an `error` event was emitted
  first), `2` bad arguments (clap usage error on stderr, no JSON).
- Stopping: the GUI pauses a run by killing the process. The CLI checkpoints
  the mapfile at least every 5 seconds and at the end of each pass, so
  re-running the same `image` command resumes from the last checkpoint with
  no loss beyond those few seconds.

Status values (used in maps): `"non_tried"`, `"non_trimmed"`,
`"non_scraped"`, `"bad_sector"`, `"finished"`.

## `sf-cli image --source S --dest D --map M [--chunk-size N] [--skip-size N] [--retries N] [--retry-only] --json`

Source starting with `\\.\` (Windows) or `/dev/` (Linux) = direct hardware
access (needs elevation); anything else = image file mode.

Events, in order:

```json
{"event":"start","source":"\\\\.\\PhysicalDrive1","mode":"device","size":2000398934016,"sector_size":512,"resumed":false}
{"event":"pass_start","pass":1,"name":"copy"}
{"event":"progress","pass":1,"current_pos":1048576,"bytes_total":2000398934016,"bytes_rescued":1048576,"bytes_bad":0,"bytes_pending":2000397885440}
{"event":"pass_complete","pass":1}
{"event":"pass_start","pass":2,"name":"retry"}
{"event":"progress","pass":2, ...same fields...}
{"event":"pass_complete","pass":2}
{"event":"done","totals":{"finished":0,"bad_sector":0,"non_trimmed":0,"non_scraped":0,"non_tried":0}}
```

- `mode` is `"device"` or `"file"`.
- `progress` is throttled to at most ~5 per second, plus one final event per
  pass. The GUI should not assume a fixed rate.
- Pass 1 is skipped (no `pass_start` 1) when `--retry-only` is given.
- On failure: `{"event":"error","message":"..."}` then exit code 1. This can
  happen before `start` (e.g. permission denied opening the device, or an
  existing mapfile whose size doesn't match the source — the CLI refuses to
  resume with the wrong map). Messages are plain English, safe to show the
  user as-is.
- Refusals before `start` (all `error` + exit 1):
  - the map has finished regions but `--dest` is missing or a different size
    (resuming would leave holes in the image);
  - **(v2)** in device mode, `--dest` or `--map` is on a volume that lives on
    the source drive. Writing to a failing drive is how data gets destroyed.
    It also refuses if it can't tell which drive an output is on.

## `sf-cli status --map M --json`

Exactly one line:

```json
{"event":"status","size":2097152,"current_pos":2097152,
 "totals":{"finished":2097152,"bad_sector":0,"non_trimmed":0,"non_scraped":0,"non_tried":0},
 "blocks":[{"pos":0,"size":2097152,"status":"finished"}]}
```

`blocks` is sorted, contiguous, and covers `[0, size)`. This is what the GUI
renders as the drive health map; it can poll this while an `image` run is in
progress (the file is replaced atomically on each checkpoint).

## `sf-cli scan --source S [--deep] --json`

Detects partitions. Reads the partition table (MBR or GPT); if there is no
usable table, or `--deep` is given, also does a full-device signature scan
for filesystem boot sectors (slow: reads the whole device).

```json
{"event":"start","source":"E:\\image.img","mode":"file","size":2000398934016,"sector_size":512}
{"event":"scan_progress","bytes_scanned":4194304,"bytes_total":2000398934016}
{"event":"result","table_kind":"mbr","partitions":[
  {"start_lba":2048,"size_sectors":1447256064,"filesystem":"ntfs","confidence":"confident","source":"mbr","label":null}
]}
```

- `scan_progress` appears only during a signature scan.
- `table_kind`: `"mbr"`, `"gpt"`, or `"none"`.
- `filesystem`: `"ntfs"`, `"fat12"`, `"fat16"`, `"fat32"`, `"exfat"`,
  `"ext"`, `"unknown"`.
- `confidence`: `"confident"` (from a valid table, boot sector verified — or
  a filesystem sitting directly at sector 0 with no table, as on many USB
  sticks; that case has `table_kind` `"none"` and `source`
  `"signature_scan"`),
  `"rebuilt"` (found by signature scan only, no table entry),
  `"low_confidence"` (table entry exists but its boot sector doesn't match a
  known filesystem).
- `source`: `"mbr"`, `"gpt"`, `"signature_scan"`.
- `size_sectors` is `null` when unknown (signature-scan results).
- `label` is the GPT partition name, else `null`.

## `sf-cli version --json` (v2)

```json
{"event":"version","version":"0.2.0","contract":2}
```

The GUI calls this at launch and warns if `contract` isn't the one it was
built for (i.e. a mismatched `sf-cli` next to it).

## `sf-cli devices --json` (v2)

Lists physical drives. Works without elevation (fields that need it may be
`null`).

```json
{"event":"devices","elevated":false,"devices":[
  {"path":"\\.\PhysicalDrive1","model":"WDC WD20EZRZ-00Z5HB0","serial":"WD-WCC4M0123456",
   "size":2000398934016,"sector_size":512,"bus":"sata","removable":false,"system":false}
]}
```

- `elevated`: whether this process could open drives for reading (Windows:
  running as Administrator; Linux: root or `disk` group). The GUI uses it to
  offer "Restart as Administrator" before the user tries a drive.
- `path` is exactly what to pass as `--source`.
- `model`, `serial`, `size`, `sector_size`: `null` when unknown.
- `bus`: `"sata"`, `"nvme"`, `"usb"`, `"scsi"`, `"sd"`, `"virtual"`, `"other"`.
- `system`: the drive holds the running OS. The GUI must make imaging it a
  deliberate choice; it can't be the failing drive you're rescuing *to*.

## Shared rules for `carve`, `files`, `extract` (v2)

- `--source` is usually the cloned image (file mode). Device mode works but
  stresses a failing drive; the GUI should steer users to the image.
- `--out DIR` is created if missing. In device mode the CLI refuses an
  `--out` on the source drive (same rule as `image --dest`).
- `--map M` (optional): the imaging mapfile for this source. Byte ranges that
  are `bad_sector` / not yet rescued in the map are treated as unreadable, so
  results can say "partial" honestly instead of silently returning zeros.
- Cancelling = killing the process. Files already written stay; a file being
  written when killed may be incomplete.
- These streams can be large (millions of `file` events on a big NTFS
  volume). The GUI must append incrementally and virtualize the list.

## `sf-cli carve --list-types --json` (v2)

```json
{"event":"types","types":[{"id":"jpg","description":"JPEG image"},{"id":"png","description":"PNG image"}]}
```

## `sf-cli carve --source S --out DIR [--types jpg,png,...] [--start B] [--end B] [--map M] --json` (v2)

Signature-based recovery from raw sectors (PhotoRec-style): finds files by
their header bytes, works with no filesystem at all, but recovered files have
no original names or folders. `--types` defaults to all; `--start`/`--end`
limit the byte range (e.g. one partition).

```json
{"event":"start","source":"E:\image.img","mode":"file","size":2000398934016,"sector_size":512}
{"event":"carve_progress","bytes_scanned":4194304,"bytes_total":2000398934016,"files_found":3}
{"event":"file","offset":1048576,"size":2381702,"type":"jpg","name":"jpg/f0002048.jpg","complete":true}
{"event":"done","files_found":3,"bytes_written":5120000}
```

- `name` is relative to `--out`, `/`-separated.
- `complete`: false when the file's end couldn't be found or it overlaps
  unreadable ranges from `--map`; the file is still written.

## `sf-cli files --source S --partition-lba N [--deleted-only] [--map M] --json` (v2)

Lists every file and folder on an NTFS volume from its MFT, including deleted
ones. `N` is the partition's `start_lba` from `scan` (512-byte units). Other
filesystems: `error` "files currently supports NTFS only".

```json
{"event":"start","source":"E:\image.img","mode":"file","size":2000398934016,"sector_size":512,"partition_lba":2048,"filesystem":"ntfs"}
{"event":"files_progress","records_read":65536,"records_total":1200000}
{"event":"file","id":4211,"parent_id":5,"name":"IMG_4821.JPG","path":"/Users/terra/Pictures/IMG_4821.JPG",
 "is_dir":false,"size":4404019,"deleted":true,"modified":"2024-03-11T09:12:00Z","created":"2024-03-11T09:12:00Z","recoverable":"full"}
{"event":"done","files":1200000,"deleted":4821}
```

- `id`: MFT record number; stable handle for `extract`. Root is `5`.
- `path`: full path from the volume root, `/`-separated; `null` if the
  parent chain is broken (orphaned file — the GUI groups these separately).
- `modified` / `created`: RFC 3339 UTC, or `null`.
- `recoverable`: `"full"` (all data readable and, for deleted files, its
  clusters are not marked in use by something else), `"partial"` (some data
  unreadable or overwritten), `"none"` (no data left, e.g. all clusters
  reused). Directories are `"full"`. NTFS-compressed or encrypted files are
  `"partial"` (their data can't be decoded yet).
- The volume root (id 5) and NTFS's own metadata files (records 0–15 such
  as `$MFT`, and everything inside `$Extend`) are not listed.

## `sf-cli extract --source S --partition-lba N (--ids 1,2,3 | --ids-file F) --out DIR [--map M] --json` (v2)

Copies files out by MFT id, recreating their folder structure under `--out`
(orphans go under `--out/_orphaned/`). Selecting a directory id extracts
everything under it. Use `--ids-file` (one id per line) for large selections;
Windows limits command lines to ~32k characters.

```json
{"event":"start","source":"E:\image.img","mode":"file","size":2000398934016,"sector_size":512,"partition_lba":2048,"filesystem":"ntfs"}
{"event":"files_progress","records_read":65536,"records_total":1200000}
{"event":"extract_progress","files_done":1,"files_total":2,"bytes_done":4404019,"bytes_total":4500000}
{"event":"extracted","id":4211,"path":"Users/terra/Pictures/IMG_4821.JPG","size":4404019,"complete":true,"note":null}
{"event":"done","extracted":2,"failed":0}
```

- `path` is relative to `--out`.
- `complete: false` + `note` (plain English) when data was unreadable,
  overwritten, or the file uses a feature not supported yet (e.g. NTFS
  compression). Whatever could be recovered is still written.
- **(v2.1 clarifications, additive)**
  - Before any `extract_progress`, the CLI reads the whole MFT (needed to
    resolve folders and paths) and emits the same `files_progress` events as
    `files` while it does.
  - Each selected id (and each file/folder under a selected folder) gets
    exactly one `extracted` event, folders included (`size` 0). If nothing
    could be written for an item (unknown id, unreadable record, output
    error), its event has `"path": null`, `"complete": false` and a `note`
    saying why, and it is counted in `done.failed` instead of
    `done.extracted`.
  - Nothing that already exists is overwritten: a name collision (e.g. a
    deleted and a live `report.docx` in the same folder) gets ` (id N)`
    before the extension. Orphans are written as
    `_orphaned/<name> (id N)<ext>`; an orphaned folder keeps its contents
    under it.
  - Names are sanitized for both Windows and Linux: `\ / : * ? " < > |` and
    control characters become `_`, trailing dots/spaces become `_`, and
    reserved names (`CON`, `aux.txt`, `COM1`…) get a leading `_`.
