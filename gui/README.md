# SectorForge GUI

Tauri 2 + React 19 + TypeScript (strict), built with Vite. Behaviour is specified in [REQUIREMENTS.md](REQUIREMENTS.md). The GUI never links `sf-core`; it spawns `sf-cli --json` and parses NDJSON (see `../docs/cli-contract.md`, contract v2).

Screens: **Image** (drive picker, live health map), **Partitions** (drive picker, scan), **Recover** (browse and extract files from NTFS including deleted ones, or carve by file signature).

## Develop

```powershell
$env:Path += ";$env:USERPROFILE\.cargo\bin"
cargo build -p sf-cli          # in the repo root; produces target\debug\sf-cli.exe
cd gui; npm install
npm run tauri dev              # real app
npm run dev                    # UI only, in a browser: a mock stands in for Rust (src/devmock.ts)
```

`npm run dev` in a plain browser is for checking layout. Add `?n=5000000` to try a huge listing, `?cli=missing` or `?cli=old` to see the sf-cli banner, `?elevated=1` to hide the Administrator prompt.

Reading a physical drive needs elevated rights. On Windows the picker offers **Restart as Administrator** (relaunches the exe with the `runas` verb and keeps the source, destination and map). On Linux device-mode jobs run through `pkexec` when the user is not root or in the `disk` group. Inside an AppImage, sf-cli is first copied to `$XDG_RUNTIME_DIR/sectorforge/` (or a private `/tmp` dir, mode 0700) because root usually cannot read the AppImage mount. **Linux device mode (pkexec, the copy, Stop) has never been exercised for real.**

## Locating sf-cli

Lookup order (`src-tauri/src/cli.rs`, `resolve`):

1. `SF_CLI_PATH` environment variable (full path; used even if missing, so the error names it)
2. the directory of the SectorForge executable (portable zip, AppImage sidecar)
3. dev: `<repo>/target/debug`, then `<repo>/target/release` (repo path baked in at compile time)
4. `sf-cli` on `PATH`

At launch the app runs `sf-cli version --json`. If sf-cli is missing, too old, or `contract` is not 2, a banner names the file it found and what to do.

## Release build

CI runs these from the repo root. Outputs: a portable Windows zip and a Linux AppImage.

### Windows (portable zip)

```powershell
cargo build --release -p sf-cli
cd gui
npm ci
npm run tauri -- build --no-bundle        # gui\src-tauri\target\release\sectorforge-gui.exe (or <repo>\target\release if a workspace target dir is used; check both)
```

The release exe has no console window (`windows_subsystem = "windows"` in `src-tauri/src/main.rs`; sf-cli is spawned with `CREATE_NO_WINDOW`).

Zip layout (both files side by side, no installer):

```
SectorForge-windows-x64.zip
  SectorForge.exe      <- sectorforge-gui.exe, renamed
  sf-cli.exe           <- target\release\sf-cli.exe
```

```powershell
$app = "gui\src-tauri\target\release\sectorforge-gui.exe"
New-Item -ItemType Directory -Force dist-zip | Out-Null
Copy-Item $app dist-zip\SectorForge.exe
Copy-Item target\release\sf-cli.exe dist-zip\sf-cli.exe
Compress-Archive dist-zip\* SectorForge-windows-x64.zip -Force
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
- `src/main.tsx` - boot: dev mock (browser only), event bridge, settings restored after elevation, first render
- `src/lib/` - framework-free logic: `api.ts` (typed Tauri commands and dialogs), `contract.ts` (event types), `filestore.ts` (column store for million-entry listings), `map.ts` (blocks to health-map cells), `devices.ts`, `recoverlogic.ts`, `format.ts`
- `src/store/` - zustand stores (`app`, `image`, `scan`, `recover`), `actions.ts` (everything the UI can do; sets "running" before invoking, rolls back on rejection), `events.ts` (sf-cli events to state), `bridge.ts` (event subscriptions, started once outside React)
- `src/components/` - shell (nav, sf-cli banner, status bar, error alert), `DrivePicker`, `HealthMap`, `VirtualList` (TanStack Virtual), `ui.tsx` primitives
- `src/screens/` - `ImageScreen`, `PartitionsScreen`, `recover/` (`RecoverScreen`, `FilesTab`, `FileBrowser`, `ExtractPanel`, `CarveTab`)
- `src/dev/devmock.ts` - browser-only stand-in for the Rust side (dev builds only; `?cli=missing|old`, `?elevated=1`, `?n=<files>`)
- `fixtures/*.ndjson` - sample CLI output; `v2_*` are hand-written from the contract (regenerate with `node scripts/gen-v2-fixtures.mjs`), `real_*` come from the actual sf-cli. Files in `../docs/contract-samples/` are parsed by the Rust tests too.

## Tests

```powershell
cd src-tauri; cargo test    # NDJSON parsing of every event, sf-cli lookup order, argument quoting
cd ..; npm test             # Vitest: store actions and events, screens (Testing Library), file store, health map, helpers
npm run build               # type check + production bundle
```
