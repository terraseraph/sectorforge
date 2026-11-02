//! Typed view of the sf-cli NDJSON contract (docs/cli-contract.md) and the
//! child-process plumbing. The GUI never links sf-core.
use serde::{Deserialize, Serialize};
use std::ffi::OsString;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

/// The contract version this GUI was built for.
pub const EXPECTED_CONTRACT: u32 = 2;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Totals {
    #[serde(default)]
    pub finished: u64,
    #[serde(default)]
    pub bad_sector: u64,
    #[serde(default)]
    pub non_trimmed: u64,
    #[serde(default)]
    pub non_scraped: u64,
    #[serde(default)]
    pub non_tried: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Block {
    pub pos: u64,
    pub size: u64,
    pub status: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Partition {
    pub start_lba: u64,
    pub size_sectors: Option<u64>,
    pub filesystem: String,
    pub confidence: String,
    pub source: String,
    pub label: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Device {
    pub path: String,
    pub model: Option<String>,
    pub serial: Option<String>,
    pub size: Option<u64>,
    pub sector_size: Option<u32>,
    #[serde(default)]
    pub bus: String,
    #[serde(default)]
    pub removable: bool,
    #[serde(default)]
    pub system: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CarveType {
    pub id: String,
    pub description: String,
}

/// The `file` event. `files` (NTFS listing) and `carve` share the event name with
/// different fields, so everything is optional and the UI picks by job.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct FileEvent {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<u64>,
    #[serde(default)]
    pub name: String,
    /// `files`: full path, or null for an orphan. Carve events have no path.
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub is_dir: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
    #[serde(default)]
    pub deleted: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub modified: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recoverable: Option<String>,
    // carve only
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offset: Option<u64>,
    #[serde(default, rename = "type", skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub complete: Option<bool>,
}

/// Unknown fields are ignored (serde default); unknown events fail to parse
/// and are logged and skipped by `parse_line`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum Event {
    Start { source: String, mode: String, size: u64, sector_size: u32, #[serde(default)] resumed: bool },
    PassStart { pass: u32, name: String },
    Progress { pass: u32, current_pos: u64, bytes_total: u64, bytes_rescued: u64, bytes_bad: u64, bytes_pending: u64 },
    PassComplete { pass: u32 },
    /// `done` differs per command: image has `totals`, carve `files_found`/`bytes_written`,
    /// files `files`/`deleted`, extract `extracted`/`failed`.
    Done {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        totals: Option<Totals>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        files_found: Option<u64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        bytes_written: Option<u64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        files: Option<u64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        deleted: Option<u64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        extracted: Option<u64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        failed: Option<u64>,
    },
    Error { message: String },
    Status { size: u64, current_pos: u64, totals: Totals, blocks: Vec<Block> },
    ScanProgress { bytes_scanned: u64, bytes_total: u64 },
    Result { table_kind: String, partitions: Vec<Partition> },
    Version { version: String, contract: u32 },
    Devices { elevated: bool, devices: Vec<Device> },
    Types { types: Vec<CarveType> },
    CarveProgress { bytes_scanned: u64, bytes_total: u64, files_found: u64 },
    FilesProgress { records_read: u64, records_total: u64 },
    File(FileEvent),
    ExtractProgress { files_done: u64, files_total: u64, bytes_done: u64, bytes_total: u64 },
    Extracted {
        id: u64,
        path: String,
        #[serde(default)]
        size: u64,
        complete: bool,
        #[serde(default)]
        note: Option<String>,
    },
}

/// Ok(None) = blank line; Err = malformed/unknown (caller logs and skips).
pub fn parse_line(line: &str) -> Result<Option<Event>, String> {
    let t = line.trim();
    if t.is_empty() {
        return Ok(None);
    }
    serde_json::from_str(t).map(Some).map_err(|e| format!("{e}: {t}"))
}

// ---- locating sf-cli ----

static ELEVATED: AtomicBool = AtomicBool::new(false);

/// Remember what `devices` reported, so Linux device jobs know whether to go through pkexec.
pub fn set_elevated(v: bool) {
    ELEVATED.store(v, Ordering::Relaxed);
}

pub fn exe_name() -> &'static str {
    if cfg!(windows) { "sf-cli.exe" } else { "sf-cli" }
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

pub fn find_in_path(name: &str, path_var: Option<OsString>) -> Option<PathBuf> {
    let path_var = path_var?;
    std::env::split_paths(&path_var).map(|d| d.join(name)).find(|p| p.is_file())
}

/// Lookup order: SF_CLI_PATH, the directory of our own executable (portable zip /
/// AppImage sidecar), dev `target/{debug,release}` (repo path baked in at compile
/// time), then PATH. All inputs are parameters so tests can exercise the order.
pub fn resolve(env: Option<OsString>, exe_dir: Option<PathBuf>, repo: &Path, path_var: Option<OsString>) -> PathBuf {
    if let Some(p) = env.filter(|p| !p.is_empty()) {
        return p.into();
    }
    let name = exe_name();
    let mut dirs: Vec<PathBuf> = exe_dir.into_iter().collect();
    dirs.extend(["debug", "release"].map(|p| repo.join("target").join(p)));
    if let Some(p) = dirs.iter().map(|d| d.join(name)).find(|p| p.is_file()) {
        return p;
    }
    find_in_path(name, path_var).unwrap_or_else(|| name.into())
}

fn our_exe_dir() -> Option<PathBuf> {
    std::env::current_exe().ok()?.parent().map(Path::to_path_buf)
}

pub fn cli_path() -> PathBuf {
    resolve(std::env::var_os("SF_CLI_PATH"), our_exe_dir(), &repo_root(), std::env::var_os("PATH"))
}

/// Where `cli_path` looks, for the "not found" banner.
pub fn searched_places() -> Vec<String> {
    let mut v = vec![match std::env::var_os("SF_CLI_PATH") {
        Some(p) if !p.is_empty() => format!("SF_CLI_PATH = {}", PathBuf::from(p).display()),
        _ => "SF_CLI_PATH (not set)".to_string(),
    }];
    if let Some(d) = our_exe_dir() {
        v.push(format!("next to SectorForge: {}", d.display()));
    }
    for p in ["debug", "release"] {
        v.push(repo_root().join("target").join(p).display().to_string());
    }
    v.push("your PATH".into());
    v
}

/// Turn the output of `sf-cli version --json` into (version, contract, problem).
pub fn interpret_version(path: &Path, stdout: &str, ran_ok: bool) -> (Option<String>, Option<u32>, Option<String>) {
    let fix = "Put the sf-cli that came in the same download next to SectorForge, or point the SF_CLI_PATH environment variable at the right file.";
    for line in stdout.lines() {
        if let Ok(Some(Event::Version { version, contract })) = parse_line(line) {
            let problem = (contract != EXPECTED_CONTRACT).then(|| {
                format!(
                    "SectorForge found sf-cli at {} (version {version}), which speaks contract {contract}, but this app needs contract {EXPECTED_CONTRACT}. {fix}",
                    path.display()
                )
            });
            return (Some(version), Some(contract), problem);
        }
    }
    let why = if ran_ok {
        "it did not answer the version check"
    } else {
        "it does not know the version command, so it is older than this app expects"
    };
    (None, None, Some(format!("SectorForge found sf-cli at {} but {why}. {fix}", path.display())))
}

/// Linux device-mode jobs run through pkexec unless we already have the rights.
pub fn wants_pkexec(args: &[String], elevated: bool, linux: bool) -> bool {
    linux && !elevated && args.windows(2).any(|w| w[0] == "--source" && w[1].starts_with("/dev/"))
}

/// Inside an AppImage the sidecar sits on a FUSE mount that root usually cannot read
/// (no allow_other), so pkexec would fail with permission denied.
pub fn needs_private_copy(cli: &Path, appdir: Option<&std::ffi::OsStr>) -> bool {
    appdir.is_some_and(|d| !d.is_empty() && cli.starts_with(d)) || cli.starts_with("/tmp/.mount_")
}

/// Where the root-readable copy lives: $XDG_RUNTIME_DIR/sectorforge, else a unique dir under /tmp.
pub fn private_dir(xdg_runtime: Option<OsString>, pid: u32, nanos: u128) -> PathBuf {
    match xdg_runtime.filter(|p| !p.is_empty()) {
        Some(x) => PathBuf::from(x).join("sectorforge"),
        None => PathBuf::from("/tmp").join(format!("sectorforge-{pid}-{nanos}")),
    }
}

/// Copy again when missing, a different size, or the source is newer than the copy.
pub fn copy_is_stale(src: (u64, std::time::SystemTime), dst: Option<(u64, std::time::SystemTime)>) -> bool {
    dst.is_none_or(|d| d.0 != src.0 || src.1 > d.1)
}

#[cfg(unix)]
fn stage_for_root(cli: &Path) -> Result<PathBuf, String> {
    use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
    static DIR: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    let err = |what: &str, e: std::io::Error| format!("Could not prepare sf-cli for administrator use ({what}): {e}");
    let dir = DIR.get_or_init(|| {
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
        private_dir(std::env::var_os("XDG_RUNTIME_DIR"), std::process::id(), nanos)
    });
    std::fs::DirBuilder::new().recursive(true).mode(0o700).create(dir).map_err(|e| err("creating folder", e))?;
    let dest = dir.join("sf-cli");
    let meta = |p: &Path| std::fs::metadata(p).ok().and_then(|m| Some((m.len(), m.modified().ok()?)));
    let src = meta(cli).ok_or_else(|| format!("Could not read sf-cli at {}.", cli.display()))?;
    if copy_is_stale(src, meta(&dest)) {
        std::fs::copy(cli, &dest).map_err(|e| err("copying", e))?;
        std::fs::set_permissions(&dest, std::fs::Permissions::from_mode(0o755)).map_err(|e| err("chmod", e))?;
    }
    Ok(dest)
}

#[cfg(not(unix))]
fn stage_for_root(cli: &Path) -> Result<PathBuf, String> {
    Ok(cli.to_path_buf())
}

pub fn command(args: &[String]) -> Command {
    let mut c = Command::new(cli_path());
    c.args(args).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    #[cfg(windows)]
    std::os::windows::process::CommandExt::creation_flags(&mut c, 0x0800_0000); // no console flash
    c
}

/// Like `command`, but wraps device-mode jobs in pkexec on Linux when not already
/// privileged. Err = readable explanation (pkexec missing).
pub fn job_command(args: &[String]) -> Result<Command, String> {
    if !wants_pkexec(args, ELEVATED.load(Ordering::Relaxed), cfg!(target_os = "linux")) {
        return Ok(command(args));
    }
    let pk = find_in_path("pkexec", std::env::var_os("PATH")).ok_or_else(|| {
        "Reading a drive directly needs administrator rights, and this system has no pkexec to ask for them. Install polkit (the pkexec command), start SectorForge with sudo, or add your user to the disk group. Or work from an image file instead.".to_string()
    })?;
    // pkexec needs an absolute program path, and root must be able to read it.
    let cli = std::fs::canonicalize(cli_path()).unwrap_or_else(|_| cli_path());
    let cli = if needs_private_copy(&cli, std::env::var_os("APPDIR").as_deref()) { stage_for_root(&cli)? } else { cli };
    let mut c = Command::new(pk);
    c.arg(cli).args(args).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    Ok(c)
}

pub fn spawn_error(e: &std::io::Error) -> String {
    let p = cli_path();
    if e.kind() == std::io::ErrorKind::NotFound {
        format!(
            "sf-cli was not found (looked for {}). Put sf-cli next to SectorForge, set the SF_CLI_PATH environment variable to its full path, or (when developing) build it with `cargo build -p sf-cli`.",
            p.display()
        )
    } else {
        format!("Could not start sf-cli ({}): {e}", p.display())
    }
}

/// Final outcome of a run, forwarded to the UI as `sf-exit`.
#[derive(Debug, Clone, Serialize)]
pub struct Exit {
    pub job: String,
    /// None when the process was killed (pause).
    pub code: Option<i32>,
    /// Stderr text (usage errors, panics) when the exit was non-zero.
    pub stderr: String,
}

/// The running child for one job kind, tagged with a run id so a finished
/// run's reader thread can never claim a newer run's process.
pub type Slot = Arc<Mutex<Option<(u64, Child)>>>;

static NEXT_RUN_ID: AtomicU64 = AtomicU64::new(1);
const STDERR_KEEP: usize = 16 * 1024;

/// Spawn, stream parsed events to `on_event`, skipped lines to `on_log`,
/// and call `on_exit` once at the end. Reading happens on background threads.
pub fn run_job(
    job: &str,
    args: Vec<String>,
    slot: Slot,
    on_event: impl Fn(Event) + Send + 'static,
    on_log: impl Fn(String) + Send + 'static,
    on_exit: impl Fn(Exit) + Send + 'static,
) -> Result<(), String> {
    let mut guard = slot.lock().unwrap();
    if guard.is_some() {
        return Err(format!("A {job} is already running."));
    }
    let mut child = job_command(&args)?.spawn().map_err(|e| spawn_error(&e))?;
    let stdout = child.stdout.take().unwrap();
    let mut stderr = child.stderr.take().unwrap();
    let run_id = NEXT_RUN_ID.fetch_add(1, Ordering::Relaxed);
    *guard = Some((run_id, child));
    drop(guard);

    // Drain stderr to EOF (a full pipe would block the child), keeping only
    // the tail for error reporting.
    let err_thread = std::thread::spawn(move || {
        let mut tail: Vec<u8> = Vec::new();
        let mut buf = [0u8; 4096];
        while let Ok(n) = stderr.read(&mut buf) {
            if n == 0 {
                break;
            }
            tail.extend_from_slice(&buf[..n]);
            if tail.len() > STDERR_KEEP {
                tail.drain(..tail.len() - STDERR_KEEP);
            }
        }
        String::from_utf8_lossy(&tail).into_owned()
    });
    let job = job.to_string();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            let Ok(line) = line else { break };
            // Stopped (or replaced): drop buffered output and close the pipe, which also
            // ends a pkexec'd child we may not be allowed to signal.
            if !slot.lock().unwrap().as_ref().is_some_and(|(id, _)| *id == run_id) {
                break;
            }
            match parse_line(&line) {
                Ok(Some(ev)) => on_event(ev),
                Ok(None) => {}
                Err(e) => on_log(format!("skipped malformed line: {e}")),
            }
        }
        let stderr_text = err_thread.join().unwrap_or_default();
        // If stop_job already took (killed) this run, code is None = paused.
        let child = {
            let mut g = slot.lock().unwrap();
            match &*g {
                Some((id, _)) if *id == run_id => g.take().map(|(_, c)| c),
                _ => None,
            }
        };
        let code = child.and_then(|mut c| c.wait().ok()).and_then(|s| s.code());
        let stderr = if code == Some(0) { String::new() } else { stderr_text };
        on_exit(Exit { job, code, stderr });
    });
    Ok(())
}

pub fn stop_job(slot: &Slot) {
    if let Some((_, mut c)) = slot.lock().unwrap().take() {
        let _ = c.kill();
        let _ = c.wait();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> String {
        std::fs::read_to_string(format!("{}/../fixtures/{name}", env!("CARGO_MANIFEST_DIR"))).unwrap()
    }
    fn parse_all(name: &str) -> (Vec<Event>, usize) {
        let (mut ok, mut bad) = (vec![], 0);
        for l in fixture(name).lines() {
            match parse_line(l) {
                Ok(Some(e)) => ok.push(e),
                Ok(None) => {}
                Err(e) => { eprintln!("{e}"); bad += 1 }
            }
        }
        (ok, bad)
    }

    #[test]
    fn image_run() {
        let (ev, bad) = parse_all("image_run.ndjson");
        assert_eq!(bad, 0);
        assert_eq!(ev.len(), 8);
        assert!(matches!(&ev[0], Event::Start { mode, size: 2000398934016, resumed: false, .. } if mode == "device"));
        assert!(matches!(&ev[5], Event::Progress { pass: 2, bytes_bad: 4096, .. }));
        assert!(matches!(&ev[7], Event::Done { totals: Some(_), .. }));
    }

    #[test]
    fn error_event() {
        let (ev, _) = parse_all("image_error.ndjson");
        assert_eq!(ev, vec![Event::Error { message: "Access is denied. (os error 5)".into() }]);
    }

    #[test]
    fn malformed_lines_are_skipped() {
        let (ev, bad) = parse_all("malformed.ndjson");
        assert_eq!(bad, 2); // garbage + unknown event; blank line is not an error
        assert_eq!(ev.len(), 2);
    }

    #[test]
    fn status_blocks() {
        let (ev, _) = parse_all("status.ndjson");
        let Event::Status { size, blocks, totals, .. } = &ev[0] else { panic!() };
        assert_eq!(*size, 2097152);
        assert_eq!(blocks.len(), 3);
        assert_eq!(blocks[1].status, "bad_sector");
        assert_eq!(totals.non_tried, 1048064);
    }

    #[test]
    fn scan_result_with_null_size() {
        let (ev, bad) = parse_all("scan.ndjson");
        assert_eq!(bad, 0);
        assert!(matches!(&ev[1], Event::ScanProgress { bytes_scanned: 4194304, .. }));
        let Event::Result { table_kind, partitions } = &ev[2] else { panic!() };
        assert_eq!(table_kind, "mbr");
        assert_eq!(partitions[1].size_sectors, None);
        assert_eq!(partitions[1].confidence, "rebuilt");
        assert_eq!(partitions[2].label.as_deref(), Some("Data"));
    }

    /// Fixture files starting with `real_` or `sample_` come from the actual sf-cli
    /// binary, so this is what proves the GUI and the CLI agree on the contract.
    /// `docs/contract-samples/` (CLI session output) is parsed too when present.
    #[test]
    fn every_line_of_real_cli_output_parses() {
        let mut dirs = vec![PathBuf::from(format!("{}/../fixtures", env!("CARGO_MANIFEST_DIR")))];
        dirs.push(PathBuf::from(format!("{}/../../docs/contract-samples", env!("CARGO_MANIFEST_DIR"))));
        let mut checked = 0;
        for dir in dirs {
            let Ok(rd) = std::fs::read_dir(&dir) else { continue };
            for entry in rd {
                let path = entry.unwrap().path();
                let name = path.file_name().unwrap().to_string_lossy().into_owned();
                let from_cli = dir.ends_with("contract-samples") || name.starts_with("real_");
                if !from_cli || !(name.ends_with(".ndjson") || name.ends_with(".jsonl") || name.ends_with(".json")) {
                    continue;
                }
                for line in std::fs::read_to_string(&path).unwrap().lines() {
                    assert!(matches!(parse_line(line), Ok(Some(_)) | Ok(None)), "{name}: {line}");
                    checked += 1;
                }
            }
        }
        assert!(checked >= 10, "expected the real_* fixtures to be present");

        let (ev, _) = parse_all("real_image_run.ndjson");
        assert!(matches!(ev.first(), Some(Event::Start { .. })));
        assert!(matches!(ev.last(), Some(Event::Done { .. })));
        let (ev, _) = parse_all("real_scan_deep.ndjson");
        let Some(Event::Result { partitions, .. }) = ev.last() else { panic!("no result") };
        assert_eq!(partitions.len(), 2);
        let (ev, _) = parse_all("real_error.ndjson");
        assert!(matches!(&ev[0], Event::Error { .. }));
    }

    #[test]
    fn serializes_with_event_tag() {
        let j = serde_json::to_value(Event::PassComplete { pass: 1 }).unwrap();
        assert_eq!(j["event"], "pass_complete");
    }

    // ---- v2 ----

    #[test]
    fn v2_version() {
        let (ev, bad) = parse_all("v2_version.ndjson");
        assert_eq!(bad, 0);
        assert_eq!(ev, vec![Event::Version { version: "0.2.0".into(), contract: 2 }]);
    }

    #[test]
    fn v2_devices_with_nulls_and_unknown_fields() {
        let (ev, bad) = parse_all("v2_devices.ndjson");
        assert_eq!(bad, 0);
        let Event::Devices { elevated, devices } = &ev[0] else { panic!() };
        assert!(!elevated);
        assert_eq!(devices.len(), 3);
        assert!(devices[0].system && devices[0].bus == "nvme");
        assert_eq!(devices[1].path, r"\\.\PhysicalDrive1");
        assert_eq!(devices[1].size, Some(2000398934016));
        assert_eq!(devices[2].model, None);
        assert_eq!(devices[2].size, None);
        assert!(devices[2].removable);
    }

    #[test]
    fn v2_types() {
        let (ev, _) = parse_all("v2_types.ndjson");
        let Event::Types { types } = &ev[0] else { panic!() };
        assert_eq!(types.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(), ["jpg", "png", "pdf"]);
        assert_eq!(types[2].description, "PDF document");
    }

    #[test]
    fn v2_carve_run() {
        let (ev, bad) = parse_all("v2_carve.ndjson");
        assert_eq!(bad, 0);
        assert!(matches!(&ev[1], Event::CarveProgress { bytes_scanned: 4194304, files_found: 3, .. }));
        let Event::File(f) = &ev[2] else { panic!() };
        assert_eq!(f.kind.as_deref(), Some("jpg"));
        assert_eq!(f.name, "jpg/f0002048.jpg");
        assert_eq!(f.offset, Some(1048576));
        assert_eq!(f.complete, Some(true));
        let Event::File(f) = &ev[3] else { panic!() };
        assert_eq!(f.complete, Some(false));
        assert!(matches!(&ev[4], Event::Done { files_found: Some(2), bytes_written: Some(2469702), totals: None, .. }));
    }

    #[test]
    fn v2_files_listing() {
        let (ev, bad) = parse_all("v2_files.ndjson");
        assert_eq!(bad, 0);
        assert!(matches!(&ev[0], Event::Start { mode, .. } if mode == "file"));
        assert!(matches!(&ev[1], Event::FilesProgress { records_read: 65536, records_total: 1200000 }));
        let Event::File(root) = &ev[2] else { panic!() };
        assert_eq!(root.id, Some(5));
        assert!(root.is_dir);
        assert_eq!(root.recoverable.as_deref(), Some("full"));
        assert_eq!(root.modified, None);
        let Event::File(f) = &ev[4] else { panic!() };
        assert_eq!(f.path.as_deref(), Some("/Users/IMG_4821.JPG"));
        assert!(f.deleted && !f.is_dir);
        assert_eq!(f.modified.as_deref(), Some("2024-03-11T09:12:00Z"));
        let Event::File(orphan) = &ev[5] else { panic!() };
        assert_eq!(orphan.path, None);
        assert_eq!(orphan.recoverable.as_deref(), Some("partial"));
        assert!(matches!(&ev[7], Event::Done { files: Some(5), deleted: Some(3), .. }));
    }

    #[test]
    fn v2_extract_run() {
        let (ev, bad) = parse_all("v2_extract.ndjson");
        assert_eq!(bad, 0);
        assert!(matches!(&ev[1], Event::ExtractProgress { files_done: 1, files_total: 2, bytes_done: 4404019, bytes_total: 4500000 }));
        assert!(matches!(&ev[2], Event::Extracted { id: 4211, complete: true, note: None, .. }));
        let Event::Extracted { complete, note, path, .. } = &ev[3] else { panic!() };
        assert!(!complete);
        assert!(note.as_deref().unwrap().contains("unreadable"));
        assert_eq!(path, "_orphaned/lost.doc");
        assert!(matches!(&ev[4], Event::Done { extracted: Some(2), failed: Some(0), .. }));
    }

    #[test]
    fn v2_image_refusal_is_an_error_event() {
        let (ev, _) = parse_all("v2_image_refusal.ndjson");
        assert!(matches!(&ev[0], Event::Error { message } if message.contains("different drive")));
    }

    #[test]
    fn file_events_serialize_compactly_and_keep_null_path() {
        let (ev, _) = parse_all("v2_files.ndjson");
        let Event::File(orphan) = &ev[5] else { panic!() };
        let j = serde_json::to_value(orphan).unwrap();
        assert!(j["path"].is_null());
        assert!(j.get("offset").is_none() && j.get("type").is_none());
        assert_eq!(j["recoverable"], "partial");
    }

    // ---- locating sf-cli ----

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("sf-gui-test-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }
    fn touch(dir: &Path) -> PathBuf {
        std::fs::create_dir_all(dir).unwrap();
        let p = dir.join(exe_name());
        std::fs::write(&p, b"x").unwrap();
        p
    }

    #[test]
    fn lookup_order() {
        let root = tmp("order");
        let (exe_dir, repo, path_dir) = (root.join("app"), root.join("repo"), root.join("onpath"));
        let dbg = touch(&repo.join("target/debug"));
        let rel = touch(&repo.join("target/release"));
        let on_path = touch(&path_dir);
        let path_var = std::env::join_paths([&path_dir]).unwrap();

        // PATH only
        let _ = std::fs::remove_file(&dbg);
        let _ = std::fs::remove_file(&rel);
        assert_eq!(resolve(None, Some(exe_dir.clone()), &repo, Some(path_var.clone())), on_path);
        // dev release beats PATH, dev debug beats release
        touch(&repo.join("target/release"));
        assert_eq!(resolve(None, Some(exe_dir.clone()), &repo, Some(path_var.clone())), rel);
        touch(&repo.join("target/debug"));
        assert_eq!(resolve(None, Some(exe_dir.clone()), &repo, Some(path_var.clone())), dbg);
        // next to our own exe beats dev dirs
        let beside = touch(&exe_dir);
        assert_eq!(resolve(None, Some(exe_dir.clone()), &repo, Some(path_var.clone())), beside);
        // SF_CLI_PATH beats everything, even if the file is missing (so the error names it)
        let forced: OsString = root.join("custom.exe").into();
        assert_eq!(resolve(Some(forced.clone()), Some(exe_dir.clone()), &repo, Some(path_var)), PathBuf::from(forced));
        // nothing anywhere: bare name, so the spawn error is NotFound
        assert_eq!(resolve(None, None, &root.join("none"), None), PathBuf::from(exe_name()));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn version_interpretation() {
        let p = Path::new("C:/x/sf-cli.exe");
        let (v, c, problem) = interpret_version(p, &fixture("v2_version.ndjson"), true);
        assert_eq!((v.as_deref(), c, problem), (Some("0.2.0"), Some(2), None));
        let (_, c, problem) = interpret_version(p, r#"{"event":"version","version":"0.1.0","contract":1}"#, true);
        assert_eq!(c, Some(1));
        let msg = problem.unwrap();
        assert!(msg.contains("sf-cli.exe") && msg.contains("contract 1") && msg.contains("needs contract 2"), "{msg}");
        // old binary: clap usage error, no JSON
        let (v, c, problem) = interpret_version(p, "", false);
        assert!(v.is_none() && c.is_none());
        assert!(problem.unwrap().contains("older"));
    }

    #[test]
    fn private_copy_decisions() {
        use std::time::{Duration, SystemTime};
        let ap = std::ffi::OsString::from("/tmp/.mount_abc");
        assert!(needs_private_copy(Path::new("/tmp/.mount_abc/usr/bin/sf-cli"), Some(&ap)));
        assert!(needs_private_copy(Path::new("/tmp/.mount_zzz/usr/bin/sf-cli"), None)); // mount seen without APPDIR
        assert!(!needs_private_copy(Path::new("/usr/bin/sf-cli"), Some(&ap)));
        assert!(!needs_private_copy(Path::new("/usr/bin/sf-cli"), None));
        assert_eq!(private_dir(Some("/run/user/1000".into()), 1, 2).to_string_lossy().replace("\\", "/"), "/run/user/1000/sectorforge");
        assert_eq!(private_dir(Some("".into()), 7, 9), PathBuf::from("/tmp/sectorforge-7-9"));
        assert_eq!(private_dir(None, 7, 9), PathBuf::from("/tmp/sectorforge-7-9"));
        let t = SystemTime::UNIX_EPOCH + Duration::from_secs(100);
        let later = t + Duration::from_secs(5);
        assert!(copy_is_stale((10, t), None));
        assert!(copy_is_stale((10, t), Some((11, later))));
        assert!(copy_is_stale((10, later), Some((10, t))));
        assert!(!copy_is_stale((10, t), Some((10, later))));
    }

    #[test]
    fn pkexec_only_for_unprivileged_linux_device_jobs() {
        let a = |s: &[&str]| s.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        let dev = a(&["image", "--source", "/dev/sdb", "--dest", "x"]);
        let file = a(&["image", "--source", "/home/u/d.img", "--dest", "x"]);
        assert!(wants_pkexec(&dev, false, true));
        assert!(!wants_pkexec(&dev, true, true));
        assert!(!wants_pkexec(&dev, false, false));
        assert!(!wants_pkexec(&file, false, true));
        // a path that merely contains /dev/ in the dest must not trigger it
        assert!(!wants_pkexec(&a(&["image", "--source", "d.img", "--dest", "/dev/null"]), false, true));
    }
}
