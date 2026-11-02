//! Debug builds only: lets the UI run in a plain browser tab (http://localhost:1420) against
//! this real, elevated app. `POST /invoke {cmd, args}` calls the same command functions as the
//! Tauri window; `GET /events` streams the same events as server-sent events.
//! Loopback only, and only pages from the Vite dev server may call it (Origin check;
//! `GET /` answers anyone with a fixed name so dev.mjs can recognise the port as ours).
//! ponytail: any local program can fake the Origin header and use the elevated engine while a
//! debug build runs. Release builds don't contain this module.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Listener};

/// `scripts/dev.mjs` picks free ports and passes them as `--bridge-port` / `--vite-port`
/// (arguments, because the elevated relaunch doesn't inherit environment variables).
const DEFAULT_PORT: u16 = 1430;
const DEFAULT_VITE_PORT: u16 = 1420;
/// Body of `GET /`, so dev.mjs can tell this bridge from another program on the port.
pub const HELLO: &str = "sectorforge-bridge";

fn port_arg(name: &str, default: u16) -> u16 {
    let args: Vec<String> = std::env::args().collect();
    args.windows(2).find(|w| w[0] == name).and_then(|w| w[1].parse().ok()).unwrap_or(default)
}

#[derive(serde::Deserialize)]
struct Req {
    cmd: String,
    #[serde(default)]
    args: serde_json::Value,
}

pub fn start(app: AppHandle) {
    let port = port_arg("--bridge-port", DEFAULT_PORT);
    let origin: Arc<str> = format!("http://localhost:{}", port_arg("--vite-port", DEFAULT_VITE_PORT)).into();
    let clients: Arc<Mutex<Vec<TcpStream>>> = Arc::default();
    for name in ["sf-event", "sf-files", "sf-log", "sf-exit"] {
        let clients = clients.clone();
        app.listen_any(name, move |e| {
            let msg = format!("event: {name}\ndata: {}\n\n", e.payload());
            clients.lock().unwrap().retain_mut(|c| c.write_all(msg.as_bytes()).is_ok());
        });
    }
    let listener = match TcpListener::bind(("127.0.0.1", port)) {
        Ok(l) => l,
        Err(e) => return eprintln!("dev bridge: can't listen on port {port}: {e}"),
    };
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let (app, clients, origin) = (app.clone(), clients.clone(), origin.clone());
            std::thread::spawn(move || {
                let _ = handle(stream, &app, &clients, &origin);
            });
        }
    });
}

fn handle(mut s: TcpStream, app: &AppHandle, clients: &Mutex<Vec<TcpStream>>, allowed: &str) -> std::io::Result<()> {
    let mut r = BufReader::new(s.try_clone()?);
    let mut line = String::new();
    r.read_line(&mut line)?;
    let mut parts = line.split_whitespace();
    let (method, path) = (parts.next().unwrap_or(""), parts.next().unwrap_or(""));
    let (mut origin, mut len) = (String::new(), 0usize);
    loop {
        let mut h = String::new();
        if r.read_line(&mut h)? == 0 || h.trim().is_empty() {
            break;
        }
        if let Some((k, v)) = h.split_once(':') {
            match k.trim().to_ascii_lowercase().as_str() {
                "origin" => origin = v.trim().to_string(),
                "content-length" => len = v.trim().parse().unwrap_or(0),
                _ => {}
            }
        }
    }
    if (method, path) == ("GET", "/") {
        let head = format!("HTTP/1.1 200 OK
Content-Length: {}
Connection: close

", HELLO.len());
        return s.write_all(format!("{head}{HELLO}").as_bytes());
    }
    // Browsers always send Origin on these cross-origin requests, so other websites are refused here.
    if origin != allowed {
        return s.write_all(b"HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
    }
    let cors = format!("Access-Control-Allow-Origin: {allowed}\r\nAccess-Control-Allow-Headers: content-type\r\n");
    match (method, path) {
        ("OPTIONS", _) => s.write_all(format!("HTTP/1.1 204 No Content\r\n{cors}Content-Length: 0\r\nConnection: close\r\n\r\n").as_bytes()),
        ("GET", "/events") => {
            s.write_all(format!("HTTP/1.1 200 OK\r\n{cors}Content-Type: text/event-stream\r\nCache-Control: no-cache\r\n\r\n").as_bytes())?;
            clients.lock().unwrap().push(s);
            Ok(())
        }
        ("POST", "/invoke") => {
            let mut body = vec![0; len.min(1 << 20)];
            r.read_exact(&mut body)?;
            let (status, out) = match serde_json::from_slice::<Req>(&body) {
                Ok(req) => match crate::bridge_invoke(app, &req.cmd, req.args) {
                    Ok(v) => ("200 OK", v),
                    Err(e) => ("400 Bad Request", e.into()),
                },
                Err(e) => ("400 Bad Request", format!("Bad request to the dev bridge: {e}").into()),
            };
            let out = out.to_string();
            let head = format!("HTTP/1.1 {status}\r\n{cors}Content-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", out.len());
            s.write_all(head.as_bytes())?;
            s.write_all(out.as_bytes())
        }
        _ => s.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"),
    }
}
