# SectorForge GUI: requirements

The desktop front end for `sf-cli`. Users point it at a **failing drive that
holds their only copy of their data**, so clarity and safety beat features:
every screen says what is happening, nothing ever writes to the source, and
results are honest about what could not be read.

## Stack

- Tauri 2 shell (`src-tauri/`, Rust). Unchanged by the React rewrite. It owns
  spawning `sf-cli --json`, parsing NDJSON, elevation, and file dialogs.
- Front end: React 19 + TypeScript (`strict`, no `any` outside the dev
  mock), Vite, `@tanstack/react-virtual` for long lists, `zustand` for
  shared state. No CSS framework: one stylesheet of design tokens.
- Tests: Vitest + Testing Library (jsdom).
- Look: dark, tokens below. Plain-English copy, no emoji, no jargon without a
  one-line explanation.

| Token | Value | Use |
|---|---|---|
| `--bg` | `#0d0f12` | page, inputs |
| `--panel` | `#15181d` | panels, nav, status bar |
| `--border` | `#242a32` | borders |
| `--text` / `--dim` | `#e8eaed` / `#8b92a0` | text / secondary |
| `--accent` | `#4fd1c5` | primary actions, focus, selection |
| `--green` / `--amber` / `--red` | `#3ecf8e` / `#e8a33d` / `#e35d5d` | good / warning or retry / bad |
| `--grey` | `#2a2f38` | not tried, empty |

## Backend API (Tauri)

Commands (`invoke`). Errors reject with a plain-English string, safe to show.

| Command | Args | Returns |
|---|---|---|
| `cli_info` | | `CliInfo` (sf-cli location, version, contract, `problem`, platform, pkexec) |
| `list_devices` | | `devices` event |
| `list_types` | | `types` event |
| `get_status` | `map` | `status` event (rejects if no map yet; normal before a first run) |
| `start_image` / `stop_image` | `a: {source, dest, map, chunkSize?, skipSize?, retries?, retryOnly?}` | |
| `start_scan` / `stop_scan` | `source, deep` | |
| `start_files` / `stop_files` | `a: {source, partitionLba, deletedOnly, map?}` | |
| `start_extract` / `stop_extract` | `a: {source, partitionLba, ids, out, map?}` | |
| `start_carve` / `stop_carve` | `a: {source, out, types[], start?, end?, map?}` (`types` empty = all) | |
| `startup_restore` | | `Restore \| null`: settings kept across "Restart as Administrator" |
| `restart_as_admin` | `restore: {source, dest, map, screen}` | Windows only; exits the app on success |

Events (`listen`):
- `sf-event` `{job, event}`: one contract event (`docs/cli-contract.md`).
  `job` is `image | scan | files | extract | carve`.
- `sf-files` `{job, files[]}`: batched `file` events (millions possible).
- `sf-log`: skipped malformed lines (console only).
- `sf-exit` `{job, code, stderr}`: `code` 0 = finished, `null` = stopped by
  us (pause/cancel), other = error.

**Ordering rule:** events can arrive before `invoke("start_*")` resolves, so
set "running" state *before* invoking, and roll it back if the invoke rejects.

Only one job runs at a time in the UI. Every start button is disabled while
any job runs, and each Stop button is enabled only for its own job.

## App shell

- Left nav: Image, Partitions, Recover (`aria-current` on the active one).
  Switching screens clears the error message.
- **sf-cli banner**: at launch call `cli_info`. If `problem` is set, show a
  red banner with the message and an expandable "Where SectorForge looked"
  list.
- **Status bar**, always visible:
  - Left: current activity in plain English ("Idle. Nothing is running.",
    "Pass 1: copying quickly...").
  - Right: access mode with its explanation. "Direct hardware access" when
    the source is `\\.\…` or `/dev/…`, "Image file" otherwise; the job's
    `start.mode` overrides once known.
- Errors: one current error message, shown in an alert at the top of the
  active screen (`role="alert"`). If a job exits non-zero without an `error`
  event, show "sf-cli stopped with exit code N." plus stderr.
- Startup order:
  1. Install the dev mock if in dev outside Tauri.
  2. Subscribe to events.
  3. Apply `startup_restore` (source/dest/map and screen).
  4. `cli_info`, then `list_devices`.
  5. Poll status once if a map path is set.
- Shared fields: **source**, **dest** and **map** are shared app state. The
  Partitions screen edits the same source as Image.

## Drive picker (Image and Partitions)

- Cards from `list_devices`: model (or "Unknown drive"), size, bus label,
  path, plus badges for "System drive" (red) and "Removable".
- Radio-group semantics: clicking a card sets the source. The selected card
  is highlighted, matching paths case-insensitively.
- States: loading, error ("Could not list drives: …"), empty (hint to type an
  image path). A Refresh button reloads.
- **Elevation notice**, when the source is a device and `elevated === false`:
  - Windows: explanation plus **Restart as Administrator**, which calls
    `restart_as_admin` with the current source/dest/map/screen. Show its
    error inline.
  - Linux with pkexec: a note that the system will ask for a password.
  - Linux without pkexec: instructions (install polkit, use sudo, or add the
    user to the `disk` group).

## Image screen

- Fields: Source (Browse: open-file dialog), Destination (Choose:
  save-file dialog), Map file (Choose: save dialog, then poll status), each
  with a hint underneath:
  - Source: `\\.\PhysicalDrive1` means direct access and needs Admin; any
    other path is an image file.
  - Destination: save it on a different drive; SectorForge refuses to write
    onto the source.
  - Map file: it records what's been copied and makes pause/resume work.
- **System drive guard**: if the source is the system drive, show a red
  notice with an "I understand, image it anyway" checkbox. Start/Resume stay
  disabled until it's ticked. The confirmation is tied to that exact source
  path.
- Advanced (collapsed): chunk size KiB (1024), skip size KiB (1024), retries
  (3). Sent in bytes.
- Buttons:
  - **Start** is disabled while running or blocked by the guard.
  - **Pause** is enabled only while imaging. It calls `stop_image` and sets
    the activity to "Paused. The map was saved; press Resume to continue."
  - **Resume** is enabled when not running, a map exists, the map isn't
    complete, and the guard isn't blocking.
- Validation: source, dest and map are all required.
- Tiles: Rescued %, Bad, Pending, Current pass. Live from `progress` while
  running, from status totals otherwise.
- **Health map**:
  - 1000 cells in 50 columns. Each cell is an equal slice of the device,
    coloured by the worst status in that slice (`cellsFromBlocks`).
  - Legend: Rescued, Needs retry, Bad sector, Not tried yet.
  - `role="img"` with a summary label.
- While imaging, poll `get_status` every 1 s, silently. Poll once more on
  exit.
- Activity text:
  - `start`: Imaging/Resuming X (size).
  - Pass 1: "copying quickly, skipping past damaged areas".
  - Pass 2: "going back to damaged areas and retrying sector by sector".
  - `done`: "Finished. Every area has been tried."
  - On exit: finished, paused or error.

## Partitions screen

- Drive picker. Source field shares the source with Image. Deep scan
  checkbox ("reads the entire drive, slow, finds partitions missing from the
  table").
- Scan (`start_scan`) and Cancel (`stop_scan`). Progress bar while
  `scan_progress` arrives.
- Results:
  - Heading: "MBR table" / "GPT table" / "no partition table".
  - Disk layout bar: segments positioned by start and size over the device
    size, coloured by confidence.
  - Table: filesystem (plus GPT label), start (bytes and LBA), size
    ("unknown" if null), confidence badge.
  - A line explaining what "Rebuilt" means. "No partitions were found." when
    the list is empty.
- `*_lba` and `*_sectors` are always 512-byte units.
- The scan result remembers which source it came from. Recover only offers it
  when the source matches.

## Recover screen

- Source field, defaulting to the Image destination until the user edits it
  (Browse opens a file). Notes underneath:
  - Whether the imaging map is in use: it is used only when the source *is*
    the Image destination and a map exists (`mapForSource`). One line on why
    that matters.
  - A warning if the source is a physical drive.
- Tabs (`role="tablist"`): **Files (NTFS)** and **Carve**.
- Before files/extract/carve read a physical drive, ask for confirmation
  (Tauri `ask` dialog): "Read the drive directly?"

### Files (NTFS)

- Partition select from the known scan result:
  - Non-NTFS options are disabled, with "(file browsing supports NTFS only)".
  - The first NTFS partition is auto-selected.
  - With no scan result, show "Press Scan" and a **Scan source** button
    (normal, not deep).
- "Only list deleted files" option. List files / Stop.
- Progress: "Reading the file table: X of Y records. N files and folders so
  far." Final, stopped and error messages follow it.
- Browser:
  - Breadcrumbs from "Volume root", plus "Orphaned files (N)" when there are
    orphans.
  - Filters: name search (debounced), Deleted only, Recoverable only. Any
    filter switches to a flat whole-partition view with a "Back to folders"
    link.
  - Count line: "N items here (M entries in all, D deleted)" or "N matches
    (of M entries)".
  - **Virtualized list** that stays smooth at 1,000,000+ entries. Each row
    has a checkbox, the name (folders are links that open them), and in
    flat view an "in <folder>" link. Then modified time (UTC), size or
    "Folder", a Deleted badge, and a recoverability badge (Fully / Partly /
    Not recoverable).
  - Header with select-all for everything currently shown, and a Clear
    selection button.
  - Selection summary, e.g. "N selected, X" or "N selected (F folders, plus
    everything inside)".
  - Folders sort first, then case-insensitive by name.
  - The store (`FileStore`) is column-oriented. React must not hold one
    object per file.
- Extract: an "Extract to" folder (Choose opens a folder dialog), then
  Extract selected and Stop.
  - Extract uses the source and partition that were *listed*, not whatever
    the fields say now.
  - It's disabled while listing, with nothing selected, or without an output
    folder. "Selecting a folder extracts everything inside it."
- Extraction panel:
  - A progress bar driven only by `extract_progress`, plus the summary.
  - A virtualized list of `extracted` rows: path (or "(not written)" when
    `null`), size, a Complete/Incomplete badge, and the note.
  - The finish summary counts extracted, failed and incomplete.

### Carve

- One line explaining carving: it finds files by their contents and works
  without a filesystem, but files come back without names.
- File-type checkboxes from `list_types` (all ticked by default), with
  Select all / none. Send `types: []` when everything is selected.
- "Search in": the whole source, or a known partition (byte range via
  `carveRange`). Includes a Scan source button.
- Save to (folder dialog). Start carving / Stop.
- Progress: "Scanned X of Y. N files found so far." Results: a virtualized
  list of type badge, saved-as name, size, position, and Complete/Incomplete.
- Finish summaries for done, stopped ("the last one may be incomplete") and
  error.

## Non-functional

- **No `dangerouslySetInnerHTML` anywhere.** File names, labels, drive models
  and error text all come from damaged disks or devices and must render as
  text. React escaping covers this, and a test asserts a hostile file name
  renders inert.
- Accessible:
  - Real buttons, labels and inputs; visible focus rings.
  - Progress bars use `role="progressbar"` with values; tabs and radio
    groups use proper roles.
  - `aria-live` on the status bar.
- Responsive enough for the window minimum of 900×640.
- `npm run build` (type check + bundle), `npm test`, and the Rust `cargo
  test` all pass. The dev mock (`?cli=missing|old`, `?elevated=1`, `?n=`)
  works in a plain browser and is never in production bundles.
