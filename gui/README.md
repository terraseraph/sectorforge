# SectorForge GUI

Tauri 2 + React 19 + TypeScript (strict), built with Vite. Behaviour is specified in [REQUIREMENTS.md](REQUIREMENTS.md). The GUI never calls `sf-core` directly; it runs the engine (`sf-cli --json`, built into the exe on Windows) as a child process and parses NDJSON (see `../docs/cli-contract.md`, contract v2).

Screens: **Image** (drive picker, live health map), **Partitions** (drive picker, scan), **Recover** (browse and extract files from NTFS including deleted ones, or carve by file signature).

## Develop

```powershell
$env:Path += ";$env:USERPROFILE\.cargo\bin"
cargo build -p sf-cli          # in the repo root; produces target\debug\sf-cli.exe
cd gui; npm install
npm run dev                    # real backend, no window; opens http://localhost:1420 in your browser
npm run dev:window             # the same app in its own window
```

Both pick free ports (Vite from 1420, the bridge from 1430), skip ports other programs hold, and stop with a message if SectorForge is already running. Both ask for Administrator (UAC) once when the backend starts. In a browser tab, debug builds serve the Tauri commands and events on 127.0.0.1:1430 (`src-tauri/src/devbridge.rs`), so the data is real; file pickers become text prompts. Release builds (`npm run tauri build`) are the single .exe with neither.

Reading a physical drive needs elevated rights. On Windows the picker offers **Restart as Administrator** (relaunches the exe with the `runas` verb and keeps the source, destination and map). On Linux device-mode jobs run through `pkexec` when the user is not root or in the `disk` group. Inside an AppImage, sf-cli is first copied to `$XDG_RUNTIME_DIR/sectorforge/` (or a private `/tmp` dir, mode 0700) because root usually cannot read the AppImage mount. **Linux device mode (pkexec, the copy, Stop) has never been exercised for real.**

## Where the engine comes from

**Windows: built in.** The GUI exe links the `sf-cli` crate as a library. For every job it launches *itself* with `sf-cli` as the first argument (`SectorForge.exe sf-cli <args>`), and `src-tauri/src/main.rs` runs the engine instead of opening a window. The JSON contract, killing the process to pause or stop, and stderr handling all work exactly as with a separate `sf-cli.exe`; there is just nothing to ship next to the exe. `SF_CLI_PATH` (a full path) still overrides this, for testing a separately built sf-cli.

**Linux: a separate `sf-cli` sidecar.** pkexec runs as root, and root can't run anything from inside the AppImage's private mount, so Linux keeps a separate static sf-cli. Lookup order (`src-tauri/src/cli.rs`, `resolve`):

1. `SF_CLI_PATH` environment variable (full path; used even if missing, so the error names it)
2. the directory of the SectorForge executable (AppImage sidecar)
3. dev: `<repo>/target/debug`, then `<repo>/target/release` (repo path baked in at compile time)
4. `sf-cli` on `PATH`

At launch the app runs `sf-cli version --json` through whichever engine applies. If it is missing, too old, or `contract` is not 2, a banner says what it found and what to do.

## Release build

CI runs these from the repo root. Outputs: one portable Windows exe and a Linux AppImage, plus standalone sf-cli builds.

### Windows (one portable exe)

```powershell
cargo build --release -p sf-cli           # only for the standalone sf-cli download
cd gui
npm ci
npx tauri build --no-bundle               # gui\src-tauri\target\release\sectorforge-gui.exe
```

Ship `sectorforge-gui.exe` renamed to `SectorForge-<version>-windows-x64.exe`. It has no console window (`windows_subsystem = "windows"`); the engine copies it starts are launched with `CREATE_NO_WINDOW` and talk over pipes. To check the engine is built in:

```powershell
.\src-tauri\target\release\sectorforge-gui.exe sf-cli version --json | Out-String   # {"contract":2,...}
```

WebView2 must be on the machine (preinstalled on Windows 11 and current Windows 10).

### Linux (AppImage, sf-cli as a Tauri sidecar)

```bash
sudo apt-get install -y libwebkit2gtk-4.1-dev libgtk-3-dev libayatana-appindicator3-dev librsvg2-dev patchelf libssl-dev
cargo build --release -p sf-cli
cd gui
npm ci
npm run stage-sidecar                      # copies target/release/sf-cli to src-tauri/binaries/sf-cli-<host triple>
npm run tauri -- build --bundles appimage  # gui/src-tauri/target/release/bundle/appimage/*.AppImage
```

`src-tauri/tauri.linux.conf.json` (merged on Linux only) sets the `appimage` target and `bundle.externalBin: ["binaries/sf-cli"]`. Tauri places the sidecar next to the app binary inside the AppImage, which is where the lookup order finds it. The sidecar entry is Linux-only on purpose: Tauri checks that the staged file exists at build time, so on Linux `cargo test` and `tauri dev` also need `npm run stage-sidecar` first. On CI runners without FUSE, set `APPIMAGE_EXTRACT_AND_RUN=1`.

`src-tauri/binaries/` is gitignored.

## Layout

- `src-tauri/src/cli.rs` - typed `Event` enum (all v2 events), line parser, sf-cli lookup, pkexec wrapping, process spawn/kill, unit tests
- `src-tauri/src/lib.rs` - Tauri commands (`start_*`/`stop_*` for image, scan, carve, files, extract; `cli_info`, `list_devices`, `list_types`, `restart_as_admin`, `startup_restore`); `file` events are batched (2000 or 80 ms) into `sf-files`; other events go as `sf-event`; process end as `sf-exit`
- `src/main.tsx` - boot: refuses to run outside the Tauri window, event bridge, settings restored after elevation, first render
- `src/lib/` - framework-free logic: `api.ts` (typed Tauri commands and dialogs), `contract.ts` (event types), `filestore.ts` (column store for million-entry listings), `map.ts` (blocks to health-map cells), `devices.ts`, `recoverlogic.ts`, `format.ts`
- `src/store/` - zustand stores (`app`, `image`, `scan`, `recover`), `actions.ts` (everything the UI can do; sets "running" before invoking, rolls back on rejection), `events.ts` (sf-cli events to state), `bridge.ts` (event subscriptions, started once outside React)
- `src/components/` - shell (nav, sf-cli banner, status bar, error alert), `DrivePicker`, `HealthMap`, `VirtualList` (TanStack Virtual), `ui.tsx` primitives
- `src/screens/` - `ImageScreen`, `PartitionsScreen`, `recover/` (`RecoverScreen`, `FilesTab`, `FileBrowser`, `ExtractPanel`, `CarveTab`)
- `fixtures/*.ndjson` - sample CLI output; `v2_*` are hand-written from the contract (regenerate with `node scripts/gen-v2-fixtures.mjs`), `real_*` come from the actual sf-cli. Files in `../docs/contract-samples/` are parsed by the Rust tests too.

## Tests

```powershell
cd src-tauri; cargo test    # NDJSON parsing of every event, sf-cli lookup order, argument quoting
cd ..; npm test             # Vitest: store actions and events, screens (Testing Library), file store, health map, helpers
npm run build               # type check + production bundle
```
