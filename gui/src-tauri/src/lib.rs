mod cli;

use cli::{Event, Slot};
use serde::Deserialize;
use tauri::{AppHandle, Emitter, State};

#[derive(Default)]
struct Jobs {
    image: Slot,
    scan: Slot,
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

fn start(app: AppHandle, slot: Slot, job: &str, args: Vec<String>) -> Result<(), String> {
    let (a, b, c) = (app.clone(), app.clone(), app);
    let jn = job.to_string();
    cli::run_job(
        job,
        args,
        slot,
        move |ev| {
            let _ = a.emit("sf-event", serde_json::json!({ "job": jn, "event": ev }));
        },
        move |msg| {
            let _ = b.emit("sf-log", msg);
        },
        move |exit| {
            let _ = c.emit("sf-exit", exit);
        },
    )
}

#[tauri::command]
fn start_image(app: AppHandle, jobs: State<Jobs>, a: ImageArgs) -> Result<(), String> {
    let mut args: Vec<String> = ["image", "--source", &a.source, "--dest", &a.dest, "--map", &a.map]
        .map(String::from)
        .into();
    for (flag, v) in [
        ("--chunk-size", a.chunk_size.map(|v| v.to_string())),
        ("--skip-size", a.skip_size.map(|v| v.to_string())),
        ("--retries", a.retries.map(|v| v.to_string())),
    ] {
        if let Some(v) = v {
            args.extend([flag.into(), v]);
        }
    }
    if a.retry_only {
        args.push("--retry-only".into());
    }
    args.push("--json".into());
    start(app, jobs.image.clone(), "image", args)
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
    start(app, jobs.scan.clone(), "scan", args)
}

#[tauri::command]
fn stop_scan(jobs: State<Jobs>) {
    cli::stop_job(&jobs.scan);
}

/// One-shot map snapshot. Err = readable message (missing binary, no map yet, ...).
#[tauri::command]
async fn get_status(map: String) -> Result<Event, String> {
    // Runs a child process to completion: keep it off the async runtime.
    let out = tauri::async_runtime::spawn_blocking(move || {
        cli::command(&["status".into(), "--map".into(), map, "--json".into()]).output()
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| cli::spawn_error(&e))?;
    let text = String::from_utf8_lossy(&out.stdout);
    for line in text.lines() {
        match cli::parse_line(line) {
            Ok(Some(Event::Error { message })) => return Err(message),
            Ok(Some(ev @ Event::Status { .. })) => return Ok(ev),
            _ => {}
        }
    }
    let err = String::from_utf8_lossy(&out.stderr);
    Err(format!("status failed ({}): {}", out.status, err.trim()))
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(Jobs::default())
        .invoke_handler(tauri::generate_handler![start_image, stop_image, start_scan, stop_scan, get_status])
        .run(tauri::generate_context!())
        .expect("error while running SectorForge");
}
