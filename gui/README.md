# SectorForge GUI

Tauri 2 + plain TypeScript/Vite. The GUI never links `sf-core`; it spawns `sf-cli --json` and parses NDJSON (see `../docs/cli-contract.md`).

## Run

```powershell
$env:Path += ";$env:USERPROFILE\.cargo\bin"
cargo build -p sf-cli          # in the repo root; produces target\debug\sf-cli.exe
cd gui; npm install
npm run tauri dev              # or: npm run tauri build -- --debug
```

Reading a physical drive (`\\.\PhysicalDriveN`) needs the **GUI** started as Administrator, since the child inherits its rights.

## Locating sf-cli

1. `SF_CLI_PATH` environment variable (full path), else
2. `<repo>/target/debug/sf-cli.exe`, then `<repo>/target/release/sf-cli.exe` (repo path baked in at compile time), else
3. `sf-cli` on `PATH`.

If none works the UI shows a message saying where it looked.

## Layout

- `src-tauri/src/cli.rs` - typed `Event` enum, line parser, process spawn/kill, unit tests
- `src-tauri/src/lib.rs` - Tauri commands: `start_image`, `stop_image`, `get_status`, `start_scan`, `stop_scan`; emits `sf-event`, `sf-log`, `sf-exit`
- `src/main.ts` - three screens + status bar; `src/map.ts` - blocks to health-map cells (worst status per cell)
- `fixtures/*.ndjson` - sample CLI output used by the tests

## Tests

```powershell
cd src-tauri; cargo test    # NDJSON parsing against fixtures
cd ..; npm test             # health-map cell logic
```
