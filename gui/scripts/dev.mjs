// Dev launcher. Picks free ports (Vite from 1420, the backend's bridge from 1430), skipping
// ports other programs hold, and refuses to start a second copy if SectorForge already runs.
//   npm run dev         real backend with no window + Vite; opens the page in your browser
//   npm run dev:window  the same app in its Tauri window (`tauri dev` with the picked ports)
// On Windows the backend asks for Administrator once. The page reaches the backend over
// src-tauri/src/devbridge.rs; ports go to it as arguments (the elevated relaunch drops env vars).
import { spawn, spawnSync } from "node:child_process";
import { writeFileSync } from "node:fs";
import { connect, createServer } from "node:net";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const windowMode = process.argv.includes("--window");
const TRIES = 20;

// Something accepts connections on this port, on either loopback address?
const answers = (port) =>
  Promise.all(
    ["127.0.0.1", "::1"].map(
      (host) =>
        new Promise((res) => {
          const s = connect(port, host, () => (s.destroy(), res(true)));
          s.on("error", () => res(false));
        }),
    ),
  ).then((r) => r.includes(true));

// Can we bind it? Catches ports Windows reserves (Hyper-V/WinNAT) that nothing answers on.
const canBind = (port) =>
  new Promise((res) => {
    const srv = createServer();
    srv.once("error", () => res(false));
    srv.listen(port, "127.0.0.1", () => srv.close(() => res(true)));
  });

async function body(url) {
  try {
    const r = await fetch(url, { signal: AbortSignal.timeout(1000) });
    return await r.text();
  } catch {
    return "";
  }
}

/** First free port from `start`, skipping `taken`. Exits if `isOurs` says SectorForge holds one. */
async function pick(start, taken, isOurs) {
  for (let port = start; port < start + TRIES; port++) {
    if (taken.includes(port)) continue;
    if (await answers(port)) {
      if (await isOurs(port)) {
        console.error(`SectorForge is already running in dev mode (port ${port} is its). Stop it with Ctrl+C in its terminal first.`);
        process.exit(1);
      }
      console.log(`Port ${port} is in use by another program, trying ${port + 1}.`);
      continue;
    }
    if (await canBind(port)) return port;
    console.log(`Port ${port} is reserved by Windows, trying ${port + 1}.`);
  }
  console.error(`No free port between ${start} and ${start + TRIES - 1}.`);
  process.exit(1);
}

const isOurVite = async (port) => (await body(`http://localhost:${port}/`)).includes("<title>SectorForge</title>");
const isOurBridge = async (port) => (await body(`http://127.0.0.1:${port}/`)) === "sectorforge-bridge";

const vitePort = await pick(1420, [], async (p) => (await isOurVite(p)) || (await isOurBridge(p)));
const bridgePort = await pick(1430, [vitePort], async (p) => (await isOurVite(p)) || (await isOurBridge(p)));
const url = `http://localhost:${vitePort}`;
const appArgs = ["--vite-port", String(vitePort), "--bridge-port", String(bridgePort)];
const env = { ...process.env, VITE_SF_BRIDGE_PORT: String(bridgePort) };
console.log(`Vite on ${vitePort}, backend bridge on ${bridgePort}.`);

let children;
if (windowMode) {
  // tauri.conf.json says 1420; this overrides it for this run only.
  const config = join(tmpdir(), "sectorforge-dev.tauri.json");
  writeFileSync(config, JSON.stringify({ build: { devUrl: url, beforeDevCommand: `npm run vite -- --port ${vitePort} --strictPort` } }));
  const tauri = join(root, "node_modules", "@tauri-apps", "cli", "tauri.js");
  children = [spawn(process.execPath, [tauri, "dev", "--config", config, "--", "--", ...appArgs], { cwd: root, stdio: "inherit", env })];
} else {
  const vite = join(root, "node_modules", "vite", "bin", "vite.js");
  children = [
    spawn("cargo", ["run", "--", "--headless", ...appArgs], { cwd: join(root, "src-tauri"), stdio: "inherit" }),
    spawn(process.execPath, [vite, "--port", String(vitePort), "--strictPort"], { cwd: root, stdio: "inherit", env }),
  ];
}

function stopAll(code) {
  for (const p of children) {
    if (p.exitCode !== null) continue;
    // Whole tree: cargo's child is the waiting backend, which takes its elevated copy with it.
    if (process.platform === "win32") spawnSync("taskkill", ["/T", "/F", "/PID", String(p.pid)], { stdio: "ignore" });
    else p.kill();
  }
  process.exit(code ?? 0);
}
for (const p of children) p.on("exit", (code) => stopAll(code));
process.on("SIGINT", () => stopAll(0));

// Browser mode: open the page once the backend answers. Cargo may compile for a while and the
// UAC prompt waits on the user, so keep trying.
if (!windowMode) {
  const timer = setInterval(async () => {
    if (!(await isOurBridge(bridgePort))) return;
    clearInterval(timer);
    console.log(`\nBackend is up. Opening ${url}`);
    const [cmd, args] = process.platform === "win32" ? ["cmd", ["/c", "start", "", url]] : [process.platform === "darwin" ? "open" : "xdg-open", [url]];
    spawn(cmd, args, { stdio: "ignore", detached: true }).unref();
  }, 500);
}
