import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open, save } from "@tauri-apps/plugin-dialog";
import { cellsFromBlocks, type Block } from "./map.ts";
import "./style.css";

// ---- contract types (docs/cli-contract.md) ----
interface Totals { finished: number; bad_sector: number; non_trimmed: number; non_scraped: number; non_tried: number }
interface Partition { start_lba: number; size_sectors: number | null; filesystem: string; confidence: string; source: string; label: string | null }
type Ev =
  | { event: "start"; source: string; mode: "device" | "file"; size: number; sector_size: number; resumed: boolean }
  | { event: "pass_start"; pass: number; name: string }
  | { event: "progress"; pass: number; current_pos: number; bytes_total: number; bytes_rescued: number; bytes_bad: number; bytes_pending: number }
  | { event: "pass_complete"; pass: number }
  | { event: "done"; totals: Totals }
  | { event: "error"; message: string }
  | { event: "status"; size: number; current_pos: number; totals: Totals; blocks: Block[] }
  | { event: "scan_progress"; bytes_scanned: number; bytes_total: number }
  | { event: "result"; table_kind: string; partitions: Partition[] };
interface Exit { job: string; code: number | null; stderr: string }

// ---- helpers ----
const esc = (s: string) => s.replace(/[&<>"']/g, (c) => `&#${c.charCodeAt(0)};`);
const $ = <T extends HTMLElement>(id: string) => document.getElementById(id) as T;
function fmt(n: number): string {
  const u = ["B", "KiB", "MiB", "GiB", "TiB"];
  let i = 0;
  while (n >= 1024 && i < u.length - 1) { n /= 1024; i++; }
  return `${i ? n.toFixed(n < 10 ? 2 : 1) : n} ${u[i]}`;
}
const isDevice = (s: string) => s.startsWith("\\\\.\\") || s.startsWith("/dev/");
const MODE_TEXT = {
  device: ["Direct hardware access", "SectorForge reads the physical drive itself. It needs Administrator rights, and it only ever reads from the drive. Reading a failing drive stresses it, so the first pass skips damage quickly."],
  file: ["Image file", "SectorForge is working on a file, not the drive, so the original hardware is not touched and the file can be re-read as often as needed."],
} as const;
const CELLS = 1000;

// ---- state ----
const st = {
  screen: "image" as "image" | "partitions" | "recover",
  running: "" as "" | "image" | "scan",
  activity: "Idle. Nothing is running.",
  mode: "" as "" | "device" | "file", // from the CLI's start event; falls back to the source field
  hasMap: false, mapDone: false,
  source: "", dest: "", map: "",
  deviceSize: 0, sectorSize: 512,
  pass: "", pollTimer: 0 as number,
  partitions: null as null | { kind: string; list: Partition[] },
  error: "",
};

// ---- shell ----
const app = $("app");
app.innerHTML = `
<div class="shell">
  <nav aria-label="Screens"><h1>SectorForge</h1>
    <button data-s="image">Image</button><button data-s="partitions">Partitions</button><button data-s="recover">Recover</button></nav>
  <main id="main"></main>
  <footer class="status" role="status" aria-live="polite"><span id="act"></span><span class="mode" id="mode"></span></footer>
</div>`;
app.querySelectorAll<HTMLButtonElement>("nav button").forEach((b) => (b.onclick = () => go(b.dataset.s as typeof st.screen)));

function renderStatusBar() {
  $("act").textContent = st.activity;
  const m = st.mode || (st.source ? (isDevice(st.source) ? "device" : "file") : "");
  $("mode").innerHTML = m ? `Mode: <b>${MODE_TEXT[m][0]}</b>. ${MODE_TEXT[m][1]}` : "Mode: choose a source to see how SectorForge will access it.";
}
function setActivity(t: string) { st.activity = t; renderStatusBar(); }
function showError(msg: string) { st.error = msg; const e = document.getElementById("err"); if (e) { e.textContent = msg; e.hidden = !msg; } }

function go(s: typeof st.screen) {
  st.screen = s;
  app.querySelectorAll("nav button").forEach((b) => (b as HTMLElement).dataset.s === s ? b.setAttribute("aria-current", "page") : b.removeAttribute("aria-current"));
  showError("");
  ({ image: renderImage, partitions: renderPartitions, recover: renderRecover })[s]();
  renderStatusBar();
}

// ---- Image screen ----
function renderImage() {
  $("main").innerHTML = `
  <h2>Image a drive</h2>
  <p class="lead">Copy a failing drive to a safe file first, then work on the copy. Pause any time; running the same copy again picks up where it stopped.</p>
  <div class="alert" id="err" role="alert" hidden></div>
  <section class="panel" aria-label="Image settings">
    <div class="field"><label for="src">Source</label><input type="text" id="src" spellcheck="false" placeholder="\\\\.\\PhysicalDrive1 or C:\\path\\disk.img" /><button class="btn" id="bsrc">Browse</button></div>
    <p class="hint"><code>\\\\.\\PhysicalDrive1</code> = direct hardware access (needs Administrator). Any other path = image file.</p>
    <div class="field"><label for="dst">Destination</label><input type="text" id="dst" spellcheck="false" /><button class="btn" id="bdst">Choose</button></div>
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
    el.oninput = () => { st.source = $<HTMLInputElement>("src").value; st.dest = $<HTMLInputElement>("dst").value; st.map = $<HTMLInputElement>("map").value; st.mode = ""; renderStatusBar(); };
  }
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
function syncButtons() {
  const b = (id: string) => document.getElementById(id) as HTMLButtonElement;
  const run = st.running === "image";
  if (document.getElementById("start")) { b("start").disabled = !!st.running; b("pause").disabled = !run; b("resume").disabled = !!st.running || !st.hasMap || st.mapDone; }
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
  showError("");
  const kib = (id: string) => Number($<HTMLInputElement>(id).value) * 1024;
  try {
    await invoke("start_image", { a: { source: st.source, dest: st.dest, map: st.map, chunkSize: kib("chunk"), skipSize: kib("skip"), retries: Number($<HTMLInputElement>("retries").value) } });
  } catch (e) { return showError(String(e)); }
  st.running = "image"; st.mode = ""; tiles.pass = "-";
  setActivity(`Starting: opening ${st.source}...`);
  syncButtons();
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
  <section class="panel" aria-label="Scan settings">
    <div class="field"><label for="psrc">Source</label><input type="text" id="psrc" spellcheck="false" placeholder="\\\\.\\PhysicalDrive1 or an image file" /><span></span></div>
    <label><input type="checkbox" id="deep" /> Deep scan (reads the entire drive, slow, finds partitions missing from the table)</label>
    <div class="actions"><button class="btn primary" id="scan">Scan</button><button class="btn" id="cancel">Cancel</button></div>
    <div class="bar" id="pbar" style="margin-top:12px" role="progressbar" aria-label="Scan progress" aria-valuemin="0" aria-valuemax="100" hidden><div></div></div>
  </section>
  <section class="panel" id="results" hidden aria-label="Partitions found"></section>`;
  const src = $<HTMLInputElement>("psrc"); src.value = st.source;
  src.oninput = () => { st.source = src.value; st.mode = ""; renderStatusBar(); };
  $("scan").onclick = async () => {
    if (!st.source) return showError("Enter a source first.");
    showError(""); st.partitions = null; scanPct = 0;
    try { await invoke("start_scan", { source: st.source, deep: $<HTMLInputElement>("deep").checked }); }
    catch (e) { return showError(String(e)); }
    st.running = "scan"; st.mode = ""; setActivity(`Reading the partition table of ${st.source}...`); drawScan();
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

// ---- Recover (placeholder) ----
function renderRecover() {
  $("main").innerHTML = `<h2>Recover files</h2><p class="lead">Browsing and copying files out of a partition is coming.</p>
  <section class="panel"><p style="margin:0">Nothing here yet. For now: image the drive, then find its partitions. Both work on the safe copy, so the later file recovery step will too.</p></section>`;
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
    case "result": st.partitions = { kind: ev.table_kind, list: ev.partitions }; drawScan(); break;
    case "error": showError(ev.message); break;
  }
}

listen<{ job: string; event: Ev }>("sf-event", (e) => (e.payload.job === "image" ? onImageEvent : onScanEvent)(e.payload.event));
listen<string>("sf-log", (e) => console.warn("sf-cli:", e.payload));
listen<Exit>("sf-exit", async (e) => {
  const x = e.payload;
  st.running = ""; clearInterval(st.pollTimer);
  if (x.code !== null && x.code !== 0 && !st.error) showError(`sf-cli stopped with exit code ${x.code}.${x.stderr ? " " + x.stderr.trim() : ""}`);
  if (x.job === "image") { await pollStatus(true); syncButtons(); if (x.code === 0) setActivity("Idle. Last run finished."); else if (x.code === null) setActivity("Paused. The map was saved; press Resume to continue."); }
  else { setActivity(x.code === 0 ? "Idle. Scan finished." : x.code === null ? "Scan cancelled." : "Scan stopped by an error."); drawScan(); }
});

go("image");
