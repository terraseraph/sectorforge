mod cli;
#[cfg(debug_assertions)]
mod devbridge;

pub use cli::EMBEDDED_ARG;

use cli::{Event, FileEvent, Slot};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, State};

#[derive(Default)]
struct Jobs {
    image: Slot,
    scan: Slot,
    carve: Slot,
    files: Slot,
    extract: Slot,
    hash: Slot,
    survey: Slot,
}

impl Jobs {
    fn all(&self) -> [&Slot; 7] {
        [&self.image, &self.scan, &self.carve, &self.files, &self.extract, &self.hash, &self.survey]
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ImageArgs {
    source: String,
    dest: String,
    map: String,
    chunk_size: Option<u64>,
    skip_size: Option<u64>,
    retries: Option<u32>,
    #[serde(default)]
    retry_only: bool,
}

/// `file` events can number in the millions; they go to the UI in batches.
const BATCH_MAX: usize = 2000;
const BATCH_EVERY: Duration = Duration::from_millis(80);

struct Batch {
    rows: Vec<FileEvent>,
    last_flush: Instant,
}

fn start(app: AppHandle, slot: Slot, job: &str, args: Vec<String>, cleanup: Option<PathBuf>) -> Result<(), String> {
    let (a, b, c) = (app.clone(), app.clone(), app);
    let batch = Arc::new(Mutex::new(Batch { rows: Vec::new(), last_flush: Instant::now() }));
    let (batch_ev, batch_exit) = (batch.clone(), batch);
    let (jn, jn_exit, jn_log) = (job.to_string(), job.to_string(), job.to_string());
    // The exact command, for the console. Elevation (pkexec) is added later and not shown.
    let _ = a.emit("sf-cmd", serde_json::json!({ "job": job, "args": args }));
    let flush = |app: &AppHandle, job: &str, b: &mut Batch| {
        if !b.rows.is_empty() {
            let _ = app.emit("sf-files", serde_json::json!({ "job": job, "files": std::mem::take(&mut b.rows) }));
        }
        b.last_flush = Instant::now();
    };
    let res = cli::run_job(
        job,
        args,
        slot,
        move |ev| {
            let mut b = batch_ev.lock().unwrap();
            match ev {
                Event::File(f) => {
                    b.rows.push(f);
                    if b.rows.len() >= BATCH_MAX || b.last_flush.elapsed() >= BATCH_EVERY {
                        flush(&a, &jn, &mut b);
                    }
                }
                // Anything else: send pending files first so the UI sees events in order.
                other => {
                    flush(&a, &jn, &mut b);
                    let _ = a.emit("sf-event", serde_json::json!({ "job": jn, "event": other }));
                }
            }
        },
        move |line| {
            let _ = b.emit("sf-log", serde_json::json!({ "job": jn_log, "line": line }));
        },
        move |exit| {
            flush(&c, &jn_exit, &mut batch_exit.lock().unwrap());
            if let Some(p) = &cleanup {
                let _ = std::fs::remove_file(p);
            }
            let _ = c.emit("sf-exit", exit);
        },
    );
    res
}

fn push_opt(args: &mut Vec<String>, flag: &str, v: Option<String>) {
    if let Some(v) = v {
        args.extend([flag.into(), v]);
    }
}

#[tauri::command]
fn start_image(app: AppHandle, jobs: State<Jobs>, a: ImageArgs) -> Result<(), String> {
    let mut args: Vec<String> = ["image", "--source", &a.source, "--dest", &a.dest, "--map", &a.map]
        .map(String::from)
        .into();
    push_opt(&mut args, "--chunk-size", a.chunk_size.map(|v| v.to_string()));
    push_opt(&mut args, "--skip-size", a.skip_size.map(|v| v.to_string()));
    push_opt(&mut args, "--retries", a.retries.map(|v| v.to_string()));
    if a.retry_only {
        args.push("--retry-only".into());
    }
    args.push("--json".into());
    start(app, jobs.image.clone(), "image", args, None)
}

#[tauri::command]
fn stop_image(jobs: State<Jobs>) {
    cli::stop_job(&jobs.image);
}

#[tauri::command]
fn start_scan(app: AppHandle, jobs: State<Jobs>, source: String, deep: bool) -> Result<(), String> {
    let mut args: Vec<String> = ["scan", "--source", &source].map(String::from).into();
    if deep {
        args.push("--deep".into());
    }
    args.push("--json".into());
    start(app, jobs.scan.clone(), "scan", args, None)
}

#[tauri::command]
fn stop_scan(jobs: State<Jobs>) {
    cli::stop_job(&jobs.scan);
}

#[tauri::command]
fn start_hash(app: AppHandle, jobs: State<Jobs>, source: String) -> Result<(), String> {
    let args = ["hash", "--source", &source, "--json"].map(String::from).into();
    start(app, jobs.hash.clone(), "hash", args, None)
}

#[tauri::command]
fn stop_hash(jobs: State<Jobs>) {
    cli::stop_job(&jobs.hash);
}

/// Background quick scan of every drive at startup. Its own slot, so it never blocks a job the user starts.
#[tauri::command]
fn start_survey(app: AppHandle, jobs: State<Jobs>, sources: Vec<String>) -> Result<(), String> {
    let mut args = vec!["survey".to_string()];
    for s in sources {
        args.extend(["--source".to_string(), s]);
    }
    args.push("--json".into());
    start(app, jobs.survey.clone(), "survey", args, None)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CarveArgs {
    source: String,
    out: String,
    /// Empty = all types (flag omitted).
    #[serde(default)]
    types: Vec<String>,
    start: Option<u64>,
    end: Option<u64>,
    map: Option<String>,
}

#[tauri::command]
fn start_carve(app: AppHandle, jobs: State<Jobs>, a: CarveArgs) -> Result<(), String> {
    let mut args: Vec<String> = ["carve", "--source", &a.source, "--out", &a.out].map(String::from).into();
    if !a.types.is_empty() {
        push_opt(&mut args, "--types", Some(a.types.join(",")));
    }
    push_opt(&mut args, "--start", a.start.map(|v| v.to_string()));
    push_opt(&mut args, "--end", a.end.map(|v| v.to_string()));
    push_opt(&mut args, "--map", a.map);
    args.push("--json".into());
    start(app, jobs.carve.clone(), "carve", args, None)
}

#[tauri::command]
fn stop_carve(jobs: State<Jobs>) {
    cli::stop_job(&jobs.carve);
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct FilesArgs {
    source: String,
    partition_lba: u64,
    #[serde(default)]
    deleted_only: bool,
    map: Option<String>,
}

#[tauri::command]
fn start_files(app: AppHandle, jobs: State<Jobs>, a: FilesArgs) -> Result<(), String> {
    let mut args: Vec<String> = ["files", "--source", &a.source, "--partition-lba", &a.partition_lba.to_string()]
        .map(String::from)
        .into();
    if a.deleted_only {
        args.push("--deleted-only".into());
    }
    push_opt(&mut args, "--map", a.map);
    args.push("--json".into());
    start(app, jobs.files.clone(), "files", args, None)
}

#[tauri::command]
fn stop_files(jobs: State<Jobs>) {
    cli::stop_job(&jobs.files);
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ExtractArgs {
    source: String,
    partition_lba: u64,
    ids: Vec<u64>,
    out: String,
    map: Option<String>,
}

/// Selections up to this size go on the command line; bigger ones through a temp file
/// (Windows limits command lines to ~32k characters).
const IDS_ON_CMDLINE: usize = 200;

#[tauri::command]
fn start_extract(app: AppHandle, jobs: State<Jobs>, a: ExtractArgs) -> Result<(), String> {
    if a.ids.is_empty() {
        return Err("Select at least one file or folder to extract.".into());
    }
    let mut args: Vec<String> = [
        "extract", "--source", &a.source, "--partition-lba", &a.partition_lba.to_string(), "--out", &a.out,
    ]
    .map(String::from)
    .into();
    let mut cleanup = None;
    if a.ids.len() <= IDS_ON_CMDLINE {
        args.extend(["--ids".into(), a.ids.iter().map(u64::to_string).collect::<Vec<_>>().join(",")]);
    } else {
        let path = std::env::temp_dir().join(format!("sectorforge-ids-{}-{}.txt", std::process::id(), a.ids.len()));
        let mut text = String::with_capacity(a.ids.len() * 8);
        for id in &a.ids {
            text.push_str(&id.to_string());
            text.push('\n');
        }
        std::fs::write(&path, text).map_err(|e| format!("Could not write the list of files to extract ({}): {e}", path.display()))?;
        args.extend(["--ids-file".into(), path.to_string_lossy().into_owned()]);
        cleanup = Some(path);
    }
    push_opt(&mut args, "--map", a.map);
    args.push("--json".into());
    let res = start(app, jobs.extract.clone(), "extract", args, cleanup.clone());
    if res.is_err() {
        if let Some(p) = cleanup {
            let _ = std::fs::remove_file(p);
        }
    }
    res
}

#[tauri::command]
fn stop_extract(jobs: State<Jobs>) {
    cli::stop_job(&jobs.extract);
}

/// Run a short command to completion and return its first matching event.
async fn one_shot(args: Vec<String>, what: &'static str) -> Result<Event, String> {
    // Runs a child process to completion: keep it off the async runtime.
    let out = tauri::async_runtime::spawn_blocking(move || cli::command(&args).output())
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| cli::spawn_error(&e))?;
    let text = String::from_utf8_lossy(&out.stdout);
    for line in text.lines() {
        match cli::parse_line(line) {
            Ok(Some(Event::Error { message })) => return Err(message),
            Ok(Some(ev)) => return Ok(ev),
            _ => {}
        }
    }
    let err = String::from_utf8_lossy(&out.stderr);
    Err(format!("{what} failed ({}): {}", out.status, err.trim()))
}

/// One-shot map snapshot. Err = readable message (missing binary, no map yet, ...).
#[tauri::command]
async fn get_status(map: String) -> Result<Event, String> {
    match one_shot(vec!["status".into(), "--map".into(), map, "--json".into()], "status").await? {
        ev @ Event::Status { .. } => Ok(ev),
        _ => Err("status returned something unexpected".into()),
    }
}

#[derive(Serialize)]
struct CliInfo {
    /// Where sf-cli was (or would have been) run from.
    path: String,
    searched: Vec<String>,
    found: bool,
    version: Option<String>,
    contract: Option<u32>,
    /// Plain-English explanation when the banner should show; None = all good.
    problem: Option<String>,
    platform: &'static str,
    pkexec: bool,
}

/// Launch-time check: where is sf-cli, and does it speak the contract this app was built for?
#[tauri::command]
async fn cli_info() -> CliInfo {
    let path = cli::engine_display();
    let searched = cli::searched_places();
    let platform = std::env::consts::OS;
    let pkexec = cli::find_in_path("pkexec", std::env::var_os("PATH")).is_some();
    let p2 = path.clone();
    let out = tauri::async_runtime::spawn_blocking(move || cli::command(&["version".into(), "--json".into()]).output())
        .await
        .ok();
    let base = |found, version, contract, problem| CliInfo {
        path: path.clone(), searched: searched.clone(), found, version, contract, problem, platform, pkexec,
    };
    match out {
        Some(Ok(o)) => {
            let (v, c, problem) = cli::interpret_version(&p2, &String::from_utf8_lossy(&o.stdout), o.status.success());
            base(true, v, c, problem)
        }
        Some(Err(e)) => {
            let msg = if e.kind() == std::io::ErrorKind::NotFound {
                let places = base(false, None, None, None).searched.join("; ");
                format!("SectorForge could not find sf-cli, the program that does the actual reading and copying. It looked in: {places}. Put {} in the same folder as SectorForge, or set SF_CLI_PATH to its full path.", cli::exe_name())
            } else {
                cli::spawn_error(&e)
            };
            base(false, None, None, Some(msg))
        }
        None => base(false, None, None, Some("Could not run the sf-cli version check.".into())),
    }
}

#[tauri::command]
async fn list_devices() -> Result<Event, String> {
    let ev = one_shot(vec!["devices".into(), "--json".into()], "devices").await?;
    if let Event::Devices { elevated, .. } = &ev {
        cli::set_elevated(*elevated);
    }
    Ok(ev)
}

#[tauri::command]
async fn list_types() -> Result<Event, String> {
    one_shot(vec!["carve".into(), "--list-types".into(), "--json".into()], "carve --list-types").await
}

// ---- elevation ----

/// Quote one argument for a Windows command-line string (CommandLineToArgvW rules).
fn quote_arg(a: &str) -> String {
    let mut out = String::from("\"");
    let mut backslashes = 0;
    for ch in a.chars() {
        match ch {
            '\\' => backslashes += 1,
            '"' => {
                out.extend(std::iter::repeat('\\').take(backslashes * 2 + 1));
                out.push('"');
                backslashes = 0;
            }
            c => {
                out.extend(std::iter::repeat('\\').take(backslashes));
                out.push(c);
                backslashes = 0;
            }
        }
    }
    out.extend(std::iter::repeat('\\').take(backslashes * 2));
    out.push('"');
    out
}

/// What the relaunched, elevated instance should show again.
#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq)]
struct Restore {
    #[serde(default)]
    source: String,
    #[serde(default)]
    dest: String,
    #[serde(default)]
    map: String,
    #[serde(default)]
    screen: String,
}

fn restore_args(r: &Restore) -> String {
    let mut v = vec!["--elevated".to_string()];
    for (flag, val) in [("--source", &r.source), ("--dest", &r.dest), ("--map", &r.map), ("--screen", &r.screen)] {
        if !val.is_empty() {
            v.push(flag.into());
            v.push(quote_arg(val));
        }
    }
    v.join(" ")
}

fn parse_restore(args: &[String]) -> Option<Restore> {
    if !args.iter().any(|a| a == "--elevated") {
        return None;
    }
    let mut r = Restore::default();
    for w in args.windows(2) {
        let val = w[1].clone();
        match w[0].as_str() {
            "--source" => r.source = val,
            "--dest" => r.dest = val,
            "--map" => r.map = val,
            "--screen" => r.screen = val,
            _ => {}
        }
    }
    Some(r)
}

/// Set when this instance was relaunched by `restart_as_admin`.
#[tauri::command]
fn startup_restore() -> Option<Restore> {
    parse_restore(&std::env::args().skip(1).collect::<Vec<_>>())
}

/// Windows: relaunch our own exe with the `runas` verb (UAC prompt), then exit.
#[cfg(windows)]
#[tauri::command]
fn restart_as_admin(app: AppHandle, jobs: State<Jobs>, restore: Restore) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::UI::Shell::ShellExecuteW;
    use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

    if jobs.all().iter().any(|s| s.lock().unwrap().is_some()) {
        return Err("Something is still running. Pause or stop it first, then restart as Administrator.".into());
    }
    let wide = |s: &std::ffi::OsStr| s.encode_wide().chain(std::iter::once(0)).collect::<Vec<u16>>();
    let exe = std::env::current_exe().map_err(|e| format!("Could not find SectorForge's own program file: {e}"))?;
    let verb = wide("runas".as_ref());
    let file = wide(exe.as_os_str());
    let params = wide(restore_args(&restore).as_ref());
    // SAFETY: all pointers are NUL-terminated UTF-16 buffers that outlive the call.
    let r = unsafe {
        ShellExecuteW(std::ptr::null_mut(), verb.as_ptr(), file.as_ptr(), params.as_ptr(), std::ptr::null(), SW_SHOWNORMAL)
    } as isize;
    if r <= 32 {
        // 5 = access denied: the user declined the Windows permission prompt.
        return Err(if r == 5 {
            "Windows did not give SectorForge Administrator rights (the permission prompt was declined).".into()
        } else {
            format!("Could not restart as Administrator (Windows error {r}).")
        });
    }
    app.exit(0);
    Ok(())
}

/// Debug builds: run the backend with no window, for the UI in a browser tab (`npm run dev`).
const HEADLESS_ARG: &str = "--headless";
/// Added by `relaunch_elevated`: the PID of the unelevated copy waiting for us.
const PARENT_ARG: &str = "--parent-pid";

/// Exit when the process that started us exits.
#[cfg(windows)]
fn exit_with_parent(pid: u32) {
    use windows_sys::Win32::System::Threading::{OpenProcess, WaitForSingleObject, INFINITE, PROCESS_SYNCHRONIZE};
    // SAFETY: plain handle calls; a null handle (parent already gone) is checked first.
    let h = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, pid) };
    if h.is_null() {
        std::process::exit(0);
    }
    let h = h as usize; // HANDLE isn't Send; it stays valid until we exit.
    std::thread::spawn(move || {
        unsafe { WaitForSingleObject(h as _, INFINITE) };
        std::process::exit(0);
    });
}

/// Windows: reading drives needs Administrator, so an unelevated start asks once (UAC),
/// waits for the elevated copy and returns its exit code. `None` = already elevated.
/// Not a requireAdministrator manifest: that makes `cargo run` / `tauri dev` fail with
/// error 740 instead of prompting.
#[cfg(windows)]
pub fn relaunch_elevated() -> Option<std::process::ExitCode> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Foundation::{CloseHandle, GetLastError};
    use windows_sys::Win32::System::Threading::{GetExitCodeProcess, WaitForSingleObject, INFINITE};
    use windows_sys::Win32::UI::Shell::{ShellExecuteExW, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW};
    use windows_sys::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONERROR, SW_HIDE, SW_SHOWNORMAL};

    if sf_core::drives::is_elevated() {
        return None;
    }
    let wide = |s: &std::ffi::OsStr| s.encode_wide().chain(std::iter::once(0)).collect::<Vec<u16>>();
    let fail = |msg: String| {
        let (text, title) = (wide(msg.as_ref()), wide("SectorForge".as_ref()));
        // SAFETY: NUL-terminated buffers that outlive the call.
        unsafe { MessageBoxW(std::ptr::null_mut(), text.as_ptr(), title.as_ptr(), MB_ICONERROR) };
        Some(std::process::ExitCode::FAILURE)
    };
    let Ok(exe) = std::env::current_exe() else { return fail("Could not find SectorForge's own program file.".into()) };
    let mut params = std::env::args().skip(1).map(|a| quote_arg(&a)).collect::<Vec<_>>();
    // The elevated copy outlives us otherwise (Ctrl+C, `tauri dev` rebuilds): it exits when we do.
    params.extend([PARENT_ARG.to_string(), std::process::id().to_string()]);
    let params = params.join(" ");
    let (verb, file, params) = (wide("runas".as_ref()), wide(exe.as_os_str()), wide(params.as_ref()));
    // SAFETY: zeroed SHELLEXECUTEINFOW is valid; every pointer set is a live NUL-terminated buffer.
    let mut info: SHELLEXECUTEINFOW = unsafe { std::mem::zeroed() };
    info.cbSize = std::mem::size_of::<SHELLEXECUTEINFOW>() as u32;
    info.fMask = SEE_MASK_NOCLOSEPROCESS;
    info.lpVerb = verb.as_ptr();
    info.lpFile = file.as_ptr();
    info.lpParameters = params.as_ptr();
    // Headless has no window of its own; don't open a console for it either.
    info.nShow = if std::env::args().any(|a| a == HEADLESS_ARG) { SW_HIDE } else { SW_SHOWNORMAL };
    if unsafe { ShellExecuteExW(&mut info) } == 0 {
        // 1223 = ERROR_CANCELLED: the user said No to the prompt.
        return match unsafe { GetLastError() } {
            1223 => fail("SectorForge needs Administrator rights to read drives. Start it again and choose Yes.".into()),
            e => fail(format!("Could not start SectorForge as Administrator (Windows error {e}).")),
        };
    }
    let mut code = 1u32;
    // SAFETY: hProcess is a valid handle because SEE_MASK_NOCLOSEPROCESS was set and the call succeeded.
    unsafe {
        WaitForSingleObject(info.hProcess, INFINITE);
        GetExitCodeProcess(info.hProcess, &mut code);
        CloseHandle(info.hProcess);
    }
    Some(std::process::ExitCode::from(code as u8))
}

#[cfg(not(windows))]
#[tauri::command]
fn restart_as_admin(_restore: Restore) -> Result<(), String> {
    Err("Restarting as Administrator is only available on Windows. On Linux, SectorForge asks for your password (pkexec) when a job needs it.".into())
}

/// The dev bridge's way into the same commands the window invokes (keep in step with
/// `generate_handler!` below).
#[cfg(debug_assertions)]
fn bridge_invoke(app: &AppHandle, cmd: &str, args: serde_json::Value) -> Result<serde_json::Value, String> {
    use tauri::async_runtime::block_on;
    use tauri::Manager;
    fn arg<T: serde::de::DeserializeOwned>(args: &serde_json::Value, k: &str) -> Result<T, String> {
        serde_json::from_value(args.get(k).cloned().unwrap_or_default()).map_err(|e| format!("Bad `{k}` argument: {e}"))
    }
    fn to<T: Serialize>(v: T) -> serde_json::Value {
        serde_json::to_value(v).unwrap_or_default()
    }
    let jobs = || app.state::<Jobs>();
    let a = app.clone();
    Ok(match cmd {
        "cli_info" => to(block_on(cli_info())),
        "list_devices" => to(block_on(list_devices())?),
        "list_types" => to(block_on(list_types())?),
        "get_status" => to(block_on(get_status(arg(&args, "map")?))?),
        "startup_restore" => to(startup_restore()),
        "start_image" => to(start_image(a, jobs(), arg(&args, "a")?)?),
        "stop_image" => to(stop_image(jobs())),
        "start_scan" => to(start_scan(a, jobs(), arg(&args, "source")?, arg(&args, "deep")?)?),
        "stop_scan" => to(stop_scan(jobs())),
        "start_hash" => to(start_hash(a, jobs(), arg(&args, "source")?)?),
        "stop_hash" => to(stop_hash(jobs())),
        "start_survey" => to(start_survey(a, jobs(), arg(&args, "sources")?)?),
        "start_carve" => to(start_carve(a, jobs(), arg(&args, "a")?)?),
        "stop_carve" => to(stop_carve(jobs())),
        "start_files" => to(start_files(a, jobs(), arg(&args, "a")?)?),
        "stop_files" => to(stop_files(jobs())),
        "start_extract" => to(start_extract(a, jobs(), arg(&args, "a")?)?),
        "stop_extract" => to(stop_extract(jobs())),
        _ => return Err(format!("`{cmd}` is not available from a browser tab.")),
    })
}

pub fn run() {
    let args: Vec<String> = std::env::args().collect();
    #[cfg(windows)]
    if let Some(pid) = args.windows(2).find(|w| w[0] == PARENT_ARG).and_then(|w| w[1].parse().ok()) {
        exit_with_parent(pid);
    }
    let headless = cfg!(debug_assertions) && args.iter().any(|a| a == HEADLESS_ARG);
    // An elevated and a normal process must not share one WebView2 profile folder.
    #[cfg(windows)]
    if std::env::args().any(|a| a == "--elevated") {
        if let Some(base) = std::env::var_os("LOCALAPPDATA") {
            let dir = PathBuf::from(base).join("io.github.terraseraph.sectorforge").join("EBWebView-admin");
            std::env::set_var("WEBVIEW2_USER_DATA_FOLDER", dir);
        }
    }
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(Jobs::default())
        .setup(move |app| {
            #[cfg(debug_assertions)]
            devbridge::start(app.handle().clone());
            // The window is `"create": false` in tauri.conf.json so headless can skip it.
            if !headless {
                tauri::WebviewWindowBuilder::from_config(app.handle(), &app.config().app.windows[0])?.build()?;
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            start_image, stop_image, start_scan, stop_scan, start_hash, stop_hash, start_survey, get_status,
            start_carve, stop_carve, start_files, stop_files, start_extract, stop_extract,
            cli_info, list_devices, list_types, startup_restore, restart_as_admin
        ])
        .run(tauri::generate_context!())
        .expect("error while running SectorForge");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quoting_follows_windows_rules() {
        assert_eq!(quote_arg(r"C:\a b\img.img"), r#""C:\a b\img.img""#);
        // trailing backslash must not swallow the closing quote
        assert_eq!(quote_arg(r"D:\out\"), r#""D:\out\\""#);
        assert_eq!(quote_arg(r#"a"b"#), r#""a\"b""#);
        assert_eq!(quote_arg(""), r#""""#);
    }

    #[test]
    fn restore_round_trip() {
        let r = Restore { source: r"\\.\PhysicalDrive1".into(), dest: r"E:\rescue\".into(), map: "".into(), screen: "image".into() };
        let line = restore_args(&r);
        assert!(line.starts_with("--elevated "));
        assert!(!line.contains("--map"));
        // simulate the OS splitting the line back into argv
        let argv: Vec<String> = ["--elevated", "--source", r"\\.\PhysicalDrive1", "--dest", r"E:\rescue\", "--screen", "image"]
            .map(String::from)
            .into();
        assert_eq!(parse_restore(&argv), Some(r));
        assert_eq!(parse_restore(&["--source".into(), "x".into()]), None);
    }
}
