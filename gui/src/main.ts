import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open, save } from "@tauri-apps/plugin-dialog";
import type { CliInfo, Ev, Exit, FileRow, Job, Restore, Totals } from "./contract.ts";
import { systemDriveFor } from "./devices.ts";
import { cellsFromBlocks, type Block } from "./map.ts";
import { mountPicker, refreshPicker } from "./picker.ts";
import * as recover from "./recover.ts";
import { beginScan, loadDevices, renderStatusBar, setActivity, showError, st, type Screen } from "./state.ts";
import { $, esc, fmt, isDevice } from "./util.ts";
import "./style.css";

const CELLS = 1000;

// ---- shell ----
const app = $("app");
app.innerHTML = `
<div class="shell">
  <nav aria-label="Screens"><h1>SectorForge</h1>
    <button data-s="image">Image</button><button data-s="partitions">Partitions</button><button data-s="recover">Recover</button></nav>
  <div class="content"><div id="banner" class="banner" role="alert" hidden></div><main id="main"></main></div>
  <footer class="status" role="status" aria-live="polite"><span id="act"></span><span class="mode" id="mode"></span></footer>
</div>`;
app.querySelectorAll<HTMLButtonElement>("nav button").forEach((b) => (b.onclick = () => go(b.dataset.s as Screen)));

function go(s: Screen) {
  st.screen = s;
  app.querySelectorAll("nav button").forEach((b) => (b as HTMLElement).dataset.s === s ? b.setAttribute("aria-current", "page") : b.removeAttribute("aria-current"));
  showError("");
  ({ image: renderImage, partitions: renderPartitions, recover: recover.renderRecover })[s]();
  renderStatusBar();
}

/** Launch check: where is sf-cli and does it speak the contract this app was built for? */
function drawBanner() {
  const b = $("banner"), c = st.cli;
  b.hidden = !c?.problem;
  if (!c?.problem) return;
  b.innerHTML = `<b>sf-cli problem.</b> ${esc(c.problem)}
    <details><summary>Where SectorForge looked</summary><ul>${c.searched.map((p) => `<li class="mono">${esc(p)}</li>`).join("")}</ul></details>`;
}
async function checkCli() {
  try { st.cli = (await invoke("cli_info")) as CliInfo; }
  catch (e) { st.cli = { path: "", searched: [], found: false, version: null, contract: null, problem: "Could not check sf-cli: " + String(e), platform: "", pkexec: true }; }
  drawBanner();
}

// ---- Image screen ----
function renderImage() {
  $("main").innerHTML = `
  <h2>Image a drive</h2>
  <p class="lead">Copy a failing drive to a safe file first, then work on the copy. Pause any time; running the same copy again picks up where it stopped.</p>
  <div class="alert" id="err" role="alert" hidden></div>
  <div id="picker"></div>
  <section class="panel" aria-label="Image settings">
    <div class="field"><label for="src">Source</label><input type="text" id="src" spellcheck="false" placeholder="\\\\.\\PhysicalDrive1 or C:\\path\\disk.img" /><button class="btn" id="bsrc">Browse</button></div>
    <p class="hint"><code>\\\\.\\PhysicalDrive1</code> = direct hardware access (needs Administrator). Any other path = image file.</p>
    <div id="sysbox"></div>
    <div class="field"><label for="dst">Destination</label><input type="text" id="dst" spellcheck="false" /><button class="btn" id="bdst">Choose</button></div>
    <p class="hint">Save the image on a different drive from the one you are rescuing. SectorForge refuses to write onto the source drive.</p>
    <div class="field"><label for="map">Map file</label><input type="text" id="map" spellcheck="false" /><button class="btn" id="bmap">Choose</button></div>
    <p class="hint">The map file records what has been copied. Keep it with the destination; it is what makes pause and resume work.</p>
    <details><summary>Advanced</summary>
      <div class="adv">
        <div><label for="chunk">Chunk size (KiB)</label><input type="number" id="chunk" min="1" value="1024" /></div>
        <div><label for="skip">Skip size (KiB)</label><input type="number" id="skip" min="1" value="1024" /></div>
        <div><label for="retries">Retries per sector</label><input type="number" id="retries" min="0" value="3" /></div>
      </div></details>
    <div class="actions">
      <button class="btn primary" id="start">Start</button><button class="btn" id="pause">Pause</button><button class="btn" id="resume">Resume</button>
    </div>
  </section>
  <div class="tiles">
    <div class="tile"><div class="k">Rescued</div><div class="v" id="t-pct">-</div></div>
    <div class="tile"><div class="k">Bad</div><div class="v" id="t-bad">-</div></div>
    <div class="tile"><div class="k">Pending</div><div class="v" id="t-pend">-</div></div>
    <div class="tile"><div class="k">Current pass</div><div class="v" id="t-pass">-</div></div>
  </div>
  <section class="panel" aria-label="Drive health map"><h3>Drive health map</h3>
    <div class="map" id="map-grid" role="img" aria-label="Drive health map: no data yet"></div>
    <div class="legend"><span style="--c:var(--green)">Rescued</span><span style="--c:var(--amber)">Needs retry</span><span style="--c:var(--red)">Bad sector</span><span style="--c:var(--grey)">Not tried yet</span></div>
    <p class="dim" style="margin:8px 0 0;font-size:12.5px">Each square is an equal slice of the drive; its colour is the worst thing found in that slice.</p>
  </section>`;
  for (const k of ["src", "dst", "map"] as const) {
    const el = $<HTMLInputElement>(k);
    el.value = { src: st.source, dst: st.dest, map: st.map }[k];
    el.oninput = () => { st.source = $<HTMLInputElement>("src").value; st.dest = $<HTMLInputElement>("dst").value; st.map = $<HTMLInputElement>("map").value; st.mode = ""; renderStatusBar(); refreshPicker(); drawSysBox(); syncButtons(); };
  }
  mountPicker($("picker"), (path) => pick("src", path));
  drawSysBox();
  $("map").onchange = () => pollStatus(true);
  $("bsrc").onclick = async () => pick("src", await open({ title: "Choose a source image file" }));
  $("bdst").onclick = async () => pick("dst", await save({ title: "Destination image file" }));
  $("bmap").onclick = async () => { pick("map", await save({ title: "Map file", defaultPath: st.map || undefined })); pollStatus(true); };
  $("start").onclick = startImage; $("resume").onclick = startImage;
  $("pause").onclick = () => { invoke("stop_image"); setActivity("Paused. The map was saved; press Resume to continue."); };
  syncButtons(); drawMap(lastStatus); drawTiles();
  if (st.error) showError(st.error);
}
function pick(id: string, v: string | string[] | null) {
  if (typeof v !== "string") return;
  const el = $<HTMLInputElement>(id); el.value = v; el.dispatchEvent(new Event("input"));
}
/** Imaging the drive the OS runs from needs a deliberate, explicit yes. */
function systemBlock() {
  const d = systemDriveFor(st.devices, st.source);
  return d && st.systemConfirmed !== st.source.trim().toLowerCase() ? d : undefined;
}
function drawSysBox() {
  const box = document.getElementById("sysbox");
  if (!box) return;
  const d = systemDriveFor(st.devices, st.source);
  if (!d) { box.innerHTML = ""; return; }
  const ok = st.systemConfirmed === st.source.trim().toLowerCase();
  box.innerHTML = `<div class="notice danger"><p><b>This is your system drive.</b> It holds the operating system SectorForge is running on, so it is in constant use and cannot be copied cleanly while you are using it. A failing drive you are rescuing is normally a different one.</p>
    <label class="check"><input type="checkbox" id="sysok"${ok ? " checked" : ""} /> I understand, and I want to image the system drive anyway</label></div>`;
  $<HTMLInputElement>("sysok").onchange = (e) => {
    st.systemConfirmed = (e.target as HTMLInputElement).checked ? st.source.trim().toLowerCase() : "";
    syncButtons();
  };
}
function syncButtons() {
  const b = (id: string) => document.getElementById(id) as HTMLButtonElement;
  const run = st.running === "image";
  const blocked = !!systemBlock();
  if (document.getElementById("start")) { b("start").disabled = !!st.running || blocked; b("pause").disabled = !run; b("resume").disabled = !!st.running || !st.hasMap || st.mapDone || blocked; }
}

let lastStatus: { size: number; blocks: Block[] } | null = null;
let tiles = { pct: "-", bad: "-", pend: "-", pass: "-" };
function drawTiles() {
  if (st.screen !== "image") return;
  $("t-pct").textContent = tiles.pct; $("t-bad").textContent = tiles.bad; $("t-pend").textContent = tiles.pend; $("t-pass").textContent = tiles.pass;
}
function drawMap(s: typeof lastStatus) {
  if (st.screen !== "image") return;
  const grid = $("map-grid");
  if (grid.children.length !== CELLS) grid.replaceChildren(...Array.from({ length: CELLS }, () => document.createElement("i")));
  const cells = s && s.size > 0 ? cellsFromBlocks(s.blocks, s.size, CELLS) : new Array(CELLS).fill("non_tried");
  cells.forEach((c, i) => { (grid.children[i] as HTMLElement).className = c; });
  grid.setAttribute("aria-label", s ? `Drive health map, ${tiles.pct} rescued, ${tiles.bad} bad` : "Drive health map: no data yet");
}
function setTilesFromTotals(size: number, t: Totals) {
  const bad = t.bad_sector;
  tiles = { ...tiles, pct: size ? `${((t.finished / size) * 100).toFixed(2)}%` : "-", bad: fmt(bad), pend: fmt(Math.max(0, size - t.finished - bad)) };
}

async function pollStatus(quiet = false) {
  if (!st.map) return;
  try {
    const ev = (await invoke("get_status", { map: st.map })) as Ev;
    if (ev.event !== "status") return;
    lastStatus = { size: ev.size, blocks: ev.blocks };
    st.hasMap = true; st.mapDone = ev.totals.finished >= ev.size && ev.size > 0;
    if (!st.running) setTilesFromTotals(ev.size, ev.totals);
    drawMap(lastStatus); drawTiles(); syncButtons();
  } catch (e) {
    // No map yet is normal before the first run; only surface it when polling a live run.
    if (!quiet) showError(String(e));
  }
}

async function startImage() {
  if (!st.source || !st.dest || !st.map) return showError("Choose a source, a destination and a map file first.");
  if (systemBlock()) return showError("That is your system drive. Tick the box under the Source field if you really mean to image it.");
  showError("");
  const kib = (id: string) => Number($<HTMLInputElement>(id).value) * 1024;
  st.running = "image"; st.mode = ""; tiles.pass = "-";
  setActivity(`Starting: opening ${st.source}...`);
  syncButtons();
  try {
    await invoke("start_image", { a: { source: st.source, dest: st.dest, map: st.map, chunkSize: kib("chunk"), skipSize: kib("skip"), retries: Number($<HTMLInputElement>("retries").value) } });
  } catch (e) { st.running = ""; setActivity("Idle. Nothing is running."); syncButtons(); return showError(String(e)); }
  clearInterval(st.pollTimer);
  st.pollTimer = window.setInterval(() => pollStatus(true), 1000);
}

// ---- Partitions screen ----
let scanPct = 0;
function renderPartitions() {
  $("main").innerHTML = `
  <h2>Find partitions</h2>
  <p class="lead">Read the partition table and, if needed, search the whole drive for filesystems. Nothing is written.</p>
  <div class="alert" id="err" role="alert" hidden></div>
  <div id="picker"></div>
  <section class="panel" aria-label="Scan settings">
    <div class="field"><label for="psrc">Source</label><input type="text" id="psrc" spellcheck="false" placeholder="\\\\.\\PhysicalDrive1 or an image file" /><span></span></div>
    <label><input type="checkbox" id="deep" /> Deep scan (reads the entire drive, slow, finds partitions missing from the table)</label>
    <div class="actions"><button class="btn primary" id="scan">Scan</button><button class="btn" id="cancel">Cancel</button></div>
    <div class="bar" id="pbar" style="margin-top:12px" role="progressbar" aria-label="Scan progress" aria-valuemin="0" aria-valuemax="100" hidden><div></div></div>
  </section>
  <section class="panel" id="results" hidden aria-label="Partitions found"></section>`;
  const src = $<HTMLInputElement>("psrc"); src.value = st.source;
  src.oninput = () => { st.source = src.value; st.mode = ""; renderStatusBar(); refreshPicker(); };
  mountPicker($("picker"), (path) => { src.value = path; src.dispatchEvent(new Event("input")); });
  $("scan").onclick = async () => {
    if (!st.source) return showError("Enter a source first.");
    showError(""); scanPct = 0;
    try { await beginScan(st.source, $<HTMLInputElement>("deep").checked); }
    catch (e) { return showError(String(e)); }
    drawScan();
  };
  $("cancel").onclick = () => { invoke("stop_scan"); };
  drawScan();
}
const CONF: Record<string, [string, string]> = { confident: ["Confident", "var(--green)"], rebuilt: ["Rebuilt", "var(--amber)"], low_confidence: ["Low confidence", "var(--red)"] };
function drawScan() {
  if (st.screen !== "partitions") return;
  const bar = $("pbar"), res = $("results");
  (document.getElementById("scan") as HTMLButtonElement).disabled = !!st.running;
  (document.getElementById("cancel") as HTMLButtonElement).disabled = st.running !== "scan";
  bar.hidden = st.running !== "scan" || scanPct === 0;
  (bar.firstElementChild as HTMLElement).style.width = `${scanPct}%`; bar.setAttribute("aria-valuenow", String(Math.round(scanPct)));
  const p = st.partitions;
  res.hidden = !p;
  if (!p) return;
  // Contract: *_lba / *_sectors are always 512-byte units, whatever the device's sector size.
  const ss = 512, total = st.deviceSize;
  const rows = p.list.map((x) => {
    const [label, c] = CONF[x.confidence] ?? [esc(x.confidence), "var(--dim)"];
    return `<tr><td>${esc(x.filesystem.toUpperCase())}${x.label ? ` <span class="dim">${esc(x.label)}</span>` : ""}</td>
      <td class="m">${(x.start_lba * ss).toLocaleString()} B <span class="dim">(LBA ${x.start_lba})</span></td>
      <td class="m">${x.size_sectors === null ? "unknown" : fmt(x.size_sectors * ss)}</td>
      <td><span class="badge" style="--c:${c}">${label}</span></td></tr>`;
  }).join("");
  const segs = total ? p.list.map((x) => {
    const [, c] = CONF[x.confidence] ?? ["", "var(--dim)"];
    const left = ((x.start_lba * ss) / total) * 100, w = x.size_sectors === null ? 0 : ((x.size_sectors * ss) / total) * 100;
    return `<div style="--c:${c};left:${left}%;width:${w}%" title="${esc(x.filesystem)} at LBA ${x.start_lba}"></div>`;
  }).join("") : "";
  res.innerHTML = `<h3>Partitions (${esc(p.kind === "none" ? "no partition table" : p.kind.toUpperCase() + " table")})</h3>
    ${p.list.length ? `<div class="layout" role="img" aria-label="Disk layout: ${p.list.length} partitions">${segs}</div>
    <div class="dim" style="display:flex;justify-content:space-between;font-size:12px"><span class="mono">0</span><span class="mono">${fmt(total)}</span></div>
    <table><thead><tr><th>Filesystem</th><th>Start</th><th>Size</th><th>Confidence</th></tr></thead><tbody>${rows}</tbody></table>
    <p class="dim" style="margin-bottom:0"><b>Rebuilt</b> means the partition is missing from the drive's partition table, so SectorForge found it by recognising a filesystem on disk. It is usually real, but its exact size is not known.</p>`
      : `<p>No partitions were found.</p>`}`;
}

// ---- events from Rust ----
function onImageEvent(ev: Ev) {
  switch (ev.event) {
    case "start":
      st.mode = ev.mode; st.deviceSize = ev.size; st.sectorSize = ev.sector_size;
      setActivity(`${ev.resumed ? "Resuming" : "Imaging"} ${ev.source} (${fmt(ev.size)}).`); break;
    case "pass_start":
      tiles.pass = `${ev.pass}: ${ev.name}`; drawTiles();
      setActivity(ev.name === "copy" ? "Pass 1: copying quickly, skipping past damaged areas to come back to later."
        : `Pass ${ev.pass}: going back to damaged areas and retrying sector by sector.`); break;
    case "progress":
      tiles.pct = `${((ev.bytes_rescued / ev.bytes_total) * 100).toFixed(2)}%`; tiles.bad = fmt(ev.bytes_bad); tiles.pend = fmt(ev.bytes_pending);
      drawTiles(); break;
    case "done": setActivity("Finished. Every area has been tried."); break;
    case "error": showError(ev.message); setActivity("Stopped by an error."); break;
  }
}
function onScanEvent(ev: Ev) {
  switch (ev.event) {
    case "start": st.mode = ev.mode; st.deviceSize = ev.size; st.sectorSize = ev.sector_size; renderStatusBar(); break;
    case "scan_progress":
      scanPct = (ev.bytes_scanned / ev.bytes_total) * 100;
      setActivity(`Scanning the whole drive for filesystems: ${scanPct.toFixed(1)}% (${fmt(ev.bytes_scanned)} of ${fmt(ev.bytes_total)}).`); drawScan(); break;
    case "result": st.partitions = { kind: ev.table_kind, list: ev.partitions, source: st.scanSource.trim() }; drawScan(); break;
    case "error": showError(ev.message); break;
  }
}

type Payload = { job: Job; event: Ev };
const ROUTE: Record<Job, (ev: Ev) => void> = {
  image: onImageEvent, scan: onScanEvent,
  files: (ev) => recover.onEvent("files", ev), carve: (ev) => recover.onEvent("carve", ev), extract: (ev) => recover.onEvent("extract", ev),
};

async function main() {
  // Outside the Tauri window (plain `vite` in a browser) a mock stands in for the Rust side, for layout checks.
  if (import.meta.env.DEV && !("__TAURI_INTERNALS__" in window)) await import("./devmock.ts");
  await listen<Payload>("sf-event", (e) => ROUTE[e.payload.job](e.payload.event));
  await listen<{ job: Job; files: FileRow[] }>("sf-files", (e) => recover.onFiles(e.payload.job, e.payload.files));
  await listen<string>("sf-log", (e) => console.warn("sf-cli:", e.payload));
  await listen<Exit>("sf-exit", async (e) => {
    const x = e.payload;
    st.running = ""; clearInterval(st.pollTimer);
    if (x.code !== null && x.code !== 0 && !st.error) showError(`sf-cli stopped with exit code ${x.code}.${x.stderr ? " " + x.stderr.trim() : ""}`);
    if (x.job === "image") { await pollStatus(true); syncButtons(); if (x.code === 0) setActivity("Idle. Last run finished."); else if (x.code === null) setActivity("Paused. The map was saved; press Resume to continue."); }
    else if (x.job === "scan") { setActivity(x.code === 0 ? "Idle. Scan finished." : x.code === null ? "Scan cancelled." : "Scan stopped by an error."); drawScan(); recover.onScanDone(); }
    else recover.onExit(x);
  });

  // After "Restart as Administrator" the new process gets the old settings back.
  let screen: Screen = "image";
  try {
    const r = (await invoke("startup_restore")) as Restore | null;
    if (r) {
      st.source = r.source; st.dest = r.dest; st.map = r.map;
      if (r.screen === "partitions" || r.screen === "recover") screen = r.screen;
    }
  } catch { /* not fatal */ }
  go(screen);
  checkCli();
  loadDevices(() => { refreshPicker(); drawSysBox(); syncButtons(); });
  if (st.map) pollStatus(true);
}
main();
