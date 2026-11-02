# sf-cli JSON contract (v1)

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
