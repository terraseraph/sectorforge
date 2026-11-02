//! Typed view of the sf-cli NDJSON contract (docs/cli-contract.md) and the
//! child-process plumbing. The GUI never links sf-core.
use serde::{Deserialize, Serialize};
use std::io::{BufRead, BufReader, Read};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

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

/// Unknown fields are ignored (serde default); unknown events fail to parse
/// and are logged and skipped by `parse_line`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum Event {
    Start { source: String, mode: String, size: u64, sector_size: u32, #[serde(default)] resumed: bool },
    PassStart { pass: u32, name: String },
    Progress { pass: u32, current_pos: u64, bytes_total: u64, bytes_rescued: u64, bytes_bad: u64, bytes_pending: u64 },
    PassComplete { pass: u32 },
    Done { totals: Totals },
    Error { message: String },
    Status { size: u64, current_pos: u64, totals: Totals, blocks: Vec<Block> },
    ScanProgress { bytes_scanned: u64, bytes_total: u64 },
    Result { table_kind: String, partitions: Vec<Partition> },
}

/// Ok(None) = blank line; Err = malformed/unknown (caller logs and skips).
pub fn parse_line(line: &str) -> Result<Option<Event>, String> {
    let t = line.trim();
    if t.is_empty() {
        return Ok(None);
    }
    serde_json::from_str(t).map(Some).map_err(|e| format!("{e}: {t}"))
}

/// Binary lookup: SF_CLI_PATH env, else <repo>/target/{debug,release}/sf-cli.exe
/// (resolved from this crate's location at compile time), else `sf-cli` on PATH.
pub fn cli_path() -> PathBuf {
    if let Some(p) = std::env::var_os("SF_CLI_PATH") {
        return p.into();
    }
    let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let name = if cfg!(windows) { "sf-cli.exe" } else { "sf-cli" };
    for profile in ["debug", "release"] {
        let p = repo.join("target").join(profile).join(name);
        if p.exists() {
            return p;
        }
    }
    "sf-cli".into()
}

pub fn command(args: &[String]) -> Command {
    let mut c = Command::new(cli_path());
    c.args(args).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    #[cfg(windows)]
    std::os::windows::process::CommandExt::creation_flags(&mut c, 0x0800_0000); // no console flash
    c
}

pub fn spawn_error(e: &std::io::Error) -> String {
    let p = cli_path();
    if e.kind() == std::io::ErrorKind::NotFound {
        format!(
            "sf-cli was not found (looked for {}). Build it with `cargo build -p sf-cli` in the repo root, or set the SF_CLI_PATH environment variable to its full path.",
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
    let mut child = command(&args).spawn().map_err(|e| spawn_error(&e))?;
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
        assert!(matches!(&ev[7], Event::Done { .. }));
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

    /// `real_*.ndjson` are captured from the actual sf-cli binary, so this is
    /// what proves the GUI and the CLI agree on the contract.
    #[test]
    fn every_line_of_real_cli_output_parses() {
        let dir = format!("{}/../fixtures", env!("CARGO_MANIFEST_DIR"));
        let mut checked = 0;
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            if !name.starts_with("real_") {
                continue;
            }
            for line in std::fs::read_to_string(&path).unwrap().lines() {
                assert!(matches!(parse_line(line), Ok(Some(_)) | Ok(None)), "{name}: {line}");
                checked += 1;
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
}
