// Recover screen: browse and extract files from an NTFS volume, or carve by signature.
import { invoke } from "@tauri-apps/api/core";
import { ask, open } from "@tauri-apps/plugin-dialog";
import type { CarveType, Ev, Exit, FileRow, Job, Partition } from "./contract.ts";
import { FileStore, ORPHANS, crumbs, noFilter, REC_NAME } from "./filestore.ts";
import { carveRange, isNtfs, mapForSource, partitionLabel } from "./recoverlogic.ts";
import { beginScan, setActivity, showError, st } from "./state.ts";
import { $, esc, fmt, fmtCount, fmtDate, isDevice } from "./util.ts";
import { VList } from "./vlist.ts";

const ROW_H = 30;

interface ExRow { path: string; size: number; complete: boolean; note: string | null }
interface CarvedRow { name: string; type: string; size: number; offset: number; complete: boolean }

const rc = {
  tab: "files" as "files" | "carve",
  source: "", sourceEdited: false,
  lba: null as number | null,
  deletedOnlyRun: false,
  store: new FileStore(),
  listing: "" as "" | "running" | "done" | "stopped" | "error",
  /** What the store currently holds the listing of; extraction must use exactly this. */
  listedFor: null as null | { source: string; lba: number },
  recordsRead: 0, recordsTotal: 0, doneInfo: "",
  folder: "/",
  filter: { deletedOnly: false, recoverableOnly: false, text: "" },
  view: [] as number[],
  selected: new Set<number>(),
  out: "",
  ex: { state: "" as "" | "running" | "done" | "stopped" | "error", filesDone: 0, filesTotal: 0, bytesDone: 0, bytesTotal: 0, rows: [] as ExRow[], summary: "" },
  types: null as null | CarveType[], typesError: "",
  chosen: new Set<string>(), carvePart: "", carveOut: "",
  cv: { state: "" as "" | "running" | "done" | "stopped" | "error", scanned: 0, total: 0, found: 0, rows: [] as CarvedRow[], summary: "" },
};

let filesList: VList | null = null, exList: VList | null = null, carveList: VList | null = null;
let viewTimer = 0;

const flat = () => !noFilter(rc.filter);
const busy = () => st.running !== "";
const mapUse = () => mapForSource(rc.source, st.dest, st.map, st.hasMap);
const known = (): Partition[] | null => (st.partitions && st.partitions.source === rc.source.trim() ? st.partitions.list : null);
const el = <T extends HTMLElement>(id: string) => document.getElementById(id) as T | null;

// ---- shell ----
export function renderRecover() {
  if (!rc.sourceEdited) rc.source = st.dest;
  filesList = exList = carveList = null;
  $("main").innerHTML = `
  <h2>Recover files</h2>
  <p class="lead">Work on the safe copy, not the failing drive. Nothing here ever writes to the source.</p>
  <div class="alert" id="err" role="alert" hidden></div>
  <section class="panel" aria-label="Source">
    <div class="field"><label for="rsrc">Source</label><input type="text" id="rsrc" spellcheck="false" placeholder="The image file you made on the Image screen" /><button class="btn" id="rbrowse">Browse</button></div>
    <div id="src-notes"></div>
  </section>
  <div class="tabs" role="tablist" aria-label="Recovery method">
    <button role="tab" id="tab-files" aria-controls="tabbody">Files (NTFS)</button>
    <button role="tab" id="tab-carve" aria-controls="tabbody">Carve</button>
  </div>
  <div id="tabbody" role="tabpanel"></div>`;
  const src = $<HTMLInputElement>("rsrc");
  src.value = rc.source;
  src.oninput = () => { rc.source = src.value; rc.sourceEdited = true; st.mode = ""; drawSourceNotes(); };
  $("rbrowse").onclick = async () => {
    const v = await open({ title: "Choose an image file or device" });
    if (typeof v === "string") { src.value = v; src.dispatchEvent(new Event("input")); drawTab(); }
  };
  $("tab-files").onclick = () => setTab("files");
  $("tab-carve").onclick = () => setTab("carve");
  drawSourceNotes();
  drawTab();
}

function setTab(t: typeof rc.tab) { rc.tab = t; drawTab(); }

function drawSourceNotes() {
  const map = mapUse();
  $("src-notes").innerHTML = `
    <p class="hint flush">${map
      ? "Using the map from the Image screen, so files that overlap bad sectors are shown as partial instead of silently containing blank data."
      : "No map in use. It applies when the source is the image made on the Image screen; without it, files over damaged areas can contain blank data without a warning."}</p>
    ${isDevice(rc.source) ? `<div class="notice warn"><p><b>This is a physical drive.</b> Reading it again and again can make a failing drive worse. Image it first on the Image screen, then recover from the image.</p></div>` : ""}`;
}

function drawTab() {
  for (const t of ["files", "carve"] as const) {
    const b = $("tab-" + t);
    b.setAttribute("aria-selected", String(rc.tab === t));
    b.tabIndex = rc.tab === t ? 0 : -1;
  }
  exList = filesList = carveList = null;
  if (rc.tab === "files") drawFilesTab(); else drawCarveTab();
  showError(st.error);
  syncButtons();
}

async function confirmDevice(what: string): Promise<boolean> {
  return ask(
    `You are about to ${what} by reading the physical drive directly. Reading a failing drive repeatedly can make it worse, and it is much slower than reading an image.\n\nThe safer way is to image the drive first (Image screen) and recover from the image.`,
    { title: "Read the drive directly?", kind: "warning", okLabel: "Read the drive anyway", cancelLabel: "Cancel" },
  );
}

async function chooseFolder(title: string): Promise<string | null> {
  const v = await open({ directory: true, title });
  return typeof v === "string" ? v : null;
}

function bar(id: string, pct: number, label: string) {
  const b = el<HTMLElement>(id);
  if (!b) return;
  (b.firstElementChild as HTMLElement).style.width = `${Math.max(0, Math.min(100, pct))}%`;
  b.setAttribute("aria-valuenow", String(Math.round(pct)));
  b.setAttribute("aria-label", label);
}

function syncButtons() {
  const dis = (id: string, v: boolean) => { const b = el<HTMLButtonElement>(id); if (b) b.disabled = v; };
  dis("rscan", busy()); dis("list", busy()); dis("lstop", st.running !== "files");
  dis("extract", busy() || rc.selected.size === 0 || !rc.out || rc.listing === "running");
  dis("exstop", st.running !== "extract");
  dis("cstart", busy()); dis("cstop", st.running !== "carve");
}

// ---- Files tab ----
function drawFilesTab() {
  const parts = known();
  if (rc.lba === null || (parts && !parts.some((p) => p.start_lba === rc.lba))) rc.lba = parts?.find(isNtfs)?.start_lba ?? null;
  const opts = parts && parts.length
    ? parts.map((p) => `<option value="${p.start_lba}"${isNtfs(p) ? "" : " disabled"}${p.start_lba === rc.lba ? " selected" : ""}>${esc(partitionLabel(p, fmt))}${isNtfs(p) ? "" : " (file browsing supports NTFS only)"}</option>`).join("")
    : `<option value="">No partitions known for this source yet. Press Scan.</option>`;
  $("tabbody").innerHTML = `
  <section class="panel" aria-label="Partition to list">
    <div class="field"><label for="part">Partition</label><select id="part">${opts}</select><button class="btn" id="rscan">Scan source</button></div>
    <label class="check"><input type="checkbox" id="delrun"${rc.deletedOnlyRun ? " checked" : ""} /> Only list deleted files (quicker on very large volumes)</label>
    <div class="actions"><button class="btn primary" id="list">List files</button><button class="btn" id="lstop">Stop</button></div>
    <div class="bar" id="lbar" style="margin-top:12px" role="progressbar" aria-valuemin="0" aria-valuemax="100" hidden><div></div></div>
    <p class="hint flush" id="lprog"></p>
  </section>
  <section class="panel" id="browser" aria-label="Files found" hidden>
    <div class="crumbs" id="crumbs" role="navigation" aria-label="Folder"></div>
    <div class="filters">
      <input type="text" id="q" placeholder="Search by name" spellcheck="false" aria-label="Search by name" value="${esc(rc.filter.text)}" />
      <label class="check"><input type="checkbox" id="fdel"${rc.filter.deletedOnly ? " checked" : ""} /> Deleted only</label>
      <label class="check"><input type="checkbox" id="frec"${rc.filter.recoverableOnly ? " checked" : ""} /> Recoverable only</label>
      <span class="dim" id="vcount"></span>
    </div>
    <div class="frow fhead" aria-hidden="true"><input type="checkbox" id="selall" aria-label="Select everything shown" /><span>Name</span><span>Modified (UTC)</span><span>Size</span><span>State</span><span>Recoverable</span></div>
    <div class="listbox" id="flist"></div>
    <div class="actions between"><span id="selinfo" class="dim"></span><span><button class="btn small" id="selnone">Clear selection</button></span></div>
    <div class="field out"><label for="out">Extract to</label><input type="text" id="out" spellcheck="false" placeholder="A folder on a different drive from the one you are rescuing" /><button class="btn" id="chout">Choose</button></div>
    <div class="actions"><button class="btn primary" id="extract">Extract selected</button><button class="btn" id="exstop">Stop</button><span class="dim">Selecting a folder extracts everything inside it.</span></div>
  </section>
  <section class="panel" id="expanel" aria-label="Extraction" hidden>
    <h3>Extraction</h3>
    <div class="bar" id="xbar" role="progressbar" aria-valuemin="0" aria-valuemax="100"><div></div></div>
    <p class="hint flush" id="xprog"></p>
    <div class="listbox short" id="xlist"></div>
  </section>`;
  $("part").onchange = () => { const v = ($("part") as HTMLSelectElement).value; rc.lba = v === "" ? null : Number(v); };
  $("rscan").onclick = scanFromRecover;
  $<HTMLInputElement>("delrun").onchange = (e) => { rc.deletedOnlyRun = (e.target as HTMLInputElement).checked; };
  $("list").onclick = startListing;
  $("lstop").onclick = () => { invoke("stop_files"); };
  const out = $<HTMLInputElement>("out");
  out.value = rc.out;
  out.oninput = () => { rc.out = out.value; syncButtons(); };
  $("chout").onclick = async () => { const d = await chooseFolder("Choose where to save recovered files"); if (d) { out.value = d; rc.out = d; syncButtons(); } };
  $("extract").onclick = startExtract;
  $("exstop").onclick = () => { invoke("stop_extract"); };
  $("selnone").onclick = () => { rc.selected.clear(); filesList?.refresh(); drawSel(); };
  $<HTMLInputElement>("selall").onchange = (e) => {
    const on = (e.target as HTMLInputElement).checked;
    for (const i of rc.view) on ? rc.selected.add(i) : rc.selected.delete(i);
    filesList?.refresh(); drawSel();
  };
  let timer = 0;
  $<HTMLInputElement>("q").oninput = (e) => {
    rc.filter.text = (e.target as HTMLInputElement).value;
    clearTimeout(timer); timer = window.setTimeout(filterChanged, 200);
  };
  $<HTMLInputElement>("fdel").onchange = (e) => { rc.filter.deletedOnly = (e.target as HTMLInputElement).checked; filterChanged(); };
  $<HTMLInputElement>("frec").onchange = (e) => { rc.filter.recoverableOnly = (e.target as HTMLInputElement).checked; filterChanged(); };

  filesList = new VList($("flist"), ROW_H, renderFileRow, "Files and folders");
  filesList.rows.addEventListener("click", onFilesClick);
  filesList.rows.addEventListener("change", onFilesCheck);
  drawListing();
  drawExtract();
}

async function scanFromRecover() {
  if (!rc.source.trim()) return showError("Choose a source first.");
  showError("");
  try { await beginScan(rc.source.trim(), false); } catch (e) { return showError(String(e)); }
  syncButtons();
}

/** Called by main when a partition scan ends. */
export function onScanDone() { if (st.screen === "recover") drawTab(); }

async function startListing() {
  const source = rc.source.trim();
  if (!source) return showError("Choose a source first.");
  if (rc.lba === null) return showError("Choose an NTFS partition first. If the list is empty, press Scan source.");
  showError("");
  if (isDevice(source) && !(await confirmDevice("list files"))) return;
  const lba = rc.lba, before = { listing: rc.listing, listedFor: rc.listedFor };
  // Set up state first: events can arrive before the invoke call returns.
  st.running = "files"; st.mode = "";
  rc.store.clear(); rc.selected.clear(); rc.view = []; rc.folder = "/";
  rc.listing = "running"; rc.listedFor = { source, lba }; rc.recordsRead = rc.recordsTotal = 0; rc.doneInfo = "";
  rc.ex = { state: "", filesDone: 0, filesTotal: 0, bytesDone: 0, bytesTotal: 0, rows: [], summary: "" };
  setActivity(`Reading the file table of ${source}...`);
  drawListing(); drawExtract(); syncButtons();
  try {
    await invoke("start_files", { a: { source, partitionLba: lba, deletedOnly: rc.deletedOnlyRun, map: mapUse() ?? null } });
  } catch (e) {
    st.running = ""; rc.listing = before.listing === "running" ? "" : before.listing; rc.listedFor = before.listedFor;
    setActivity("Idle. Nothing is running."); drawListing(); syncButtons();
    return showError(String(e));
  }
}

function drawListing() {
  if (!filesList) return;
  const running = rc.listing === "running";
  const lb = $("lbar");
  lb.hidden = !running;
  bar("lbar", rc.recordsTotal ? (rc.recordsRead / rc.recordsTotal) * 100 : 0, "File table progress");
  const n = fmtCount(rc.store.count);
  $("lprog").textContent =
    running ? `Reading the file table: ${fmtCount(rc.recordsRead)} of ${rc.recordsTotal ? fmtCount(rc.recordsTotal) : "?"} records. ${n} files and folders so far.`
    : rc.listing === "done" ? rc.doneInfo
    : rc.listing === "stopped" ? `Stopped early. ${n} entries were read, so the list is incomplete.`
    : rc.listing === "error" ? `Listing failed. ${n} entries were read before it stopped.` : "";
  $("browser").hidden = rc.listing === "" ;
  filesList.setEmpty(running ? "Reading the file table..." : flat() ? "Nothing matches these filters." : "This folder is empty.");
  computeView();
}

function computeView() {
  if (!filesList) return;
  rc.view = flat() ? rc.store.search(rc.filter) : rc.store.list(rc.folder);
  filesList.setCount(rc.view.length);
  const c = $("crumbs");
  if (flat()) {
    c.innerHTML = `<span class="dim">Showing matches from the whole partition.</span> <button class="lnk" data-clear="1">Back to folders</button>`;
  } else {
    const items = crumbs(rc.folder).map((x, i, a) => {
      const last = i === a.length - 1 && rc.folder !== ORPHANS;
      return last ? `<b>${esc(x.label)}</b>` : `<button class="lnk" data-dir="${i}">${esc(x.label)}</button>`;
    });
    const orph = rc.store.orphanCount;
    c.innerHTML = items.join(`<span class="sep">/</span>`) +
      (orph ? `<span class="sep">|</span>${rc.folder === ORPHANS ? `<b>Orphaned files (${fmtCount(orph)})</b>` : `<button class="lnk" data-orph="1">Orphaned files (${fmtCount(orph)})</button>`}` : "");
  }
  c.querySelectorAll<HTMLElement>("[data-dir]").forEach((b) => (b.onclick = () => navigate(crumbs(rc.folder)[Number(b.dataset.dir)].dir)));
  c.querySelector<HTMLElement>("[data-orph]")?.addEventListener("click", () => navigate(ORPHANS));
  c.querySelector<HTMLElement>("[data-clear]")?.addEventListener("click", clearFilters);
  const total = fmtCount(rc.store.count);
  $("vcount").textContent = flat()
    ? `${fmtCount(rc.view.length)} matches (of ${total} entries)`
    : `${fmtCount(rc.view.length)} items here (${total} entries in all, ${fmtCount(rc.store.deletedCount)} deleted)`;
  const all = $<HTMLInputElement>("selall");
  all.checked = rc.view.length > 0 && rc.view.every((i) => rc.selected.has(i));
  drawSel();
}

function scheduleView() { if (!viewTimer) viewTimer = window.setTimeout(() => { viewTimer = 0; if (rc.tab === "files") { drawListing(); } }, 250); }

function navigate(dir: string) { rc.folder = dir; filesList?.scrollToTop(); computeView(); }
function filterChanged() { filesList?.scrollToTop(); computeView(); }
function clearFilters() {
  rc.filter = { deletedOnly: false, recoverableOnly: false, text: "" };
  for (const [id, v] of [["q", ""], ["fdel", false], ["frec", false]] as const) {
    const i = $<HTMLInputElement>(id); if (typeof v === "string") i.value = v; else i.checked = v;
  }
  filterChanged();
}

const REC_BADGE = {
  full: ["Fully recoverable", "var(--green)"],
  partial: ["Partly recoverable", "var(--amber)"],
  none: ["Not recoverable", "var(--red)"],
} as const;

function renderFileRow(k: number, row: HTMLElement) {
  const i = rc.view[k], s = rc.store;
  if (i === undefined) { row.innerHTML = ""; return; }
  const dir = s.isDir(i), name = esc(s.names[i]);
  const [rl, rcol] = REC_BADGE[REC_NAME[s.recov[i]]];
  const folder = s.folderOf(i);
  const where = flat() ? ` <button class="lnk dim" data-act="folder">in ${esc(folder === ORPHANS ? "Orphaned files" : folder)}</button>` : "";
  row.className = `vl-row frow${rc.selected.has(i) ? " sel" : ""}`;
  row.innerHTML = `<input type="checkbox" class="chk" ${rc.selected.has(i) ? "checked" : ""} aria-label="Select ${name}" />
    <span class="nm">${dir ? `<button class="lnk" data-act="open">${name}</button>` : name}${where}</span>
    <span class="m dim">${esc(fmtDate(s.modified[i]))}</span>
    <span class="m">${dir ? "Folder" : esc(fmt(s.sizes[i]))}</span>
    <span>${s.isDeleted(i) ? `<span class="badge" style="--c:var(--amber)">Deleted</span>` : ""}</span>
    <span><span class="badge" style="--c:${rcol}">${rl}</span></span>`;
}

function rowIndex(t: HTMLElement): number | undefined {
  const row = t.closest<HTMLElement>(".vl-row");
  return row ? rc.view[Number(row.dataset.i)] : undefined;
}
function onFilesClick(e: Event) {
  const t = (e.target as HTMLElement).closest<HTMLElement>("[data-act]");
  if (!t) return;
  const i = rowIndex(t);
  if (i === undefined) return;
  const s = rc.store;
  if (t.dataset.act === "open") { if (flat()) { clearFilters(); } navigate(s.pathOf(i)); }
  else if (t.dataset.act === "folder") { const f = s.folderOf(i); clearFilters(); navigate(f); }
}
function onFilesCheck(e: Event) {
  const t = e.target as HTMLInputElement;
  if (!t.classList.contains("chk")) return;
  const i = rowIndex(t);
  if (i === undefined) return;
  if (t.checked) rc.selected.add(i); else rc.selected.delete(i);
  t.closest(".vl-row")!.classList.toggle("sel", t.checked);
  const all = $<HTMLInputElement>("selall");
  all.checked = rc.view.length > 0 && rc.view.every((x) => rc.selected.has(x));
  drawSel();
}

function drawSel() {
  const info = el<HTMLElement>("selinfo");
  if (!info) return;
  let bytes = 0, dirs = 0;
  for (const i of rc.selected) { if (rc.store.isDir(i)) dirs++; else bytes += rc.store.sizes[i]; }
  info.textContent = rc.selected.size
    ? `${fmtCount(rc.selected.size)} selected${dirs ? ` (${fmtCount(dirs)} folders, plus everything inside)` : `, ${fmt(bytes)}`}`
    : "Nothing selected. Tick files or folders to extract.";
  syncButtons();
}

// ---- extraction ----
async function startExtract() {
  if (!rc.listedFor) return;
  const { source, lba } = rc.listedFor;
  const ids = [...rc.selected].map((i) => rc.store.ids[i]).filter((id) => id >= 0);
  if (!ids.length) return showError("Select at least one file or folder first.");
  if (!rc.out) return showError("Choose a folder to extract into.");
  showError("");
  if (isDevice(source) && !(await confirmDevice("extract files"))) return;
  st.running = "extract"; st.mode = "";
  rc.ex = { state: "running", filesDone: 0, filesTotal: ids.length, bytesDone: 0, bytesTotal: 0, rows: [], summary: "" };
  setActivity(`Extracting ${fmtCount(ids.length)} selected items to ${rc.out}...`);
  drawExtract(); syncButtons();
  try {
    await invoke("start_extract", { a: { source, partitionLba: lba, ids, out: rc.out, map: mapForSource(source, st.dest, st.map, st.hasMap) ?? null } });
  } catch (e) {
    st.running = ""; rc.ex.state = ""; setActivity("Idle. Nothing is running."); drawExtract(); syncButtons();
    return showError(String(e));
  }
}

function renderExRow(k: number, row: HTMLElement) {
  const r = rc.ex.rows[k];
  if (!r) { row.innerHTML = ""; return; }
  row.className = "vl-row xrow";
  row.innerHTML = `<span class="nm" title="${esc(r.note ?? "")}">${esc(r.path)}</span><span class="m">${esc(fmt(r.size))}</span>
    <span>${r.complete ? `<span class="badge" style="--c:var(--green)">Complete</span>` : `<span class="badge" style="--c:var(--amber)">Incomplete</span>`}</span>
    <span class="dim note">${esc(r.note ?? "")}</span>`;
}

function drawExtract() {
  const p = el<HTMLElement>("expanel");
  if (!p) return;
  p.hidden = rc.ex.state === "";
  if (rc.ex.state === "") return;
  const x = rc.ex;
  bar("xbar", x.filesTotal ? (x.filesDone / x.filesTotal) * 100 : 0, "Extraction progress");
  $("xprog").textContent = x.summary ||
    `${fmtCount(x.filesDone)} of ${fmtCount(x.filesTotal)} files, ${fmt(x.bytesDone)}${x.bytesTotal ? ` of ${fmt(x.bytesTotal)}` : ""}.`;
  if (!exList) { exList = new VList($("xlist"), ROW_H, renderExRow, "Extracted files"); exList.setEmpty("Waiting for the first file..."); }
  exList.setCount(x.rows.length);
}

// ---- Carve tab ----
function drawCarveTab() {
  const parts = known() ?? [];
  const partOpts = `<option value="">The whole source</option>` +
    parts.map((p) => `<option value="${p.start_lba}"${String(p.start_lba) === rc.carvePart ? " selected" : ""}>${esc(partitionLabel(p, fmt))}</option>`).join("");
  $("tabbody").innerHTML = `
  <section class="panel" aria-label="Carve settings">
    <p class="lead flush">Carving looks for known file types by their first bytes, so it works even with no working filesystem. Recovered files have no names or folders.</p>
    <h3>File types to look for</h3>
    <div class="types" id="types">${typesHtml()}</div>
    <div class="actions"><button class="btn small" id="tall">Select all</button><button class="btn small" id="tnone">Select none</button></div>
    <div class="field spaced"><label for="cpart">Search in</label><select id="cpart">${partOpts}</select><button class="btn" id="rscan">Scan source</button></div>
    <p class="hint">Limiting the search to one partition is faster and avoids matches from other volumes.</p>
    <div class="field"><label for="cout">Save to</label><input type="text" id="cout" spellcheck="false" placeholder="A folder on a different drive from the one you are rescuing" /><button class="btn" id="cchout">Choose</button></div>
    <div class="actions"><button class="btn primary" id="cstart">Start carving</button><button class="btn" id="cstop">Stop</button></div>
    <div class="bar" id="cbar" style="margin-top:12px" role="progressbar" aria-valuemin="0" aria-valuemax="100" hidden><div></div></div>
    <p class="hint flush" id="cprog"></p>
  </section>
  <section class="panel" id="cresults" aria-label="Files found" hidden>
    <h3>Files found</h3>
    <div class="frow fhead cv" aria-hidden="true"><span>Type</span><span>Saved as</span><span>Size</span><span>Position on source</span><span>State</span></div>
    <div class="listbox" id="clist"></div>
  </section>`;
  bindTypes();
  $("tall").onclick = () => { rc.chosen = new Set((rc.types ?? []).map((t) => t.id)); $("types").innerHTML = typesHtml(); bindTypes(); };
  $("tnone").onclick = () => { rc.chosen.clear(); $("types").innerHTML = typesHtml(); bindTypes(); };
  $("cpart").onchange = () => { rc.carvePart = ($("cpart") as HTMLSelectElement).value; };
  $("rscan").onclick = scanFromRecover;
  const out = $<HTMLInputElement>("cout");
  out.value = rc.carveOut;
  out.oninput = () => { rc.carveOut = out.value; };
  $("cchout").onclick = async () => { const d = await chooseFolder("Choose where to save carved files"); if (d) { out.value = d; rc.carveOut = d; } };
  $("cstart").onclick = startCarve;
  $("cstop").onclick = () => { invoke("stop_carve"); };
  if (!rc.types && !rc.typesError) loadTypes();
  drawCarve();
}

function typesHtml(): string {
  if (rc.typesError) return `<p class="dim">Could not load the list of file types: ${esc(rc.typesError)}</p>`;
  if (!rc.types) return `<p class="dim">Loading file types...</p>`;
  return rc.types.map((t) => `<label class="check"><input type="checkbox" data-type="${esc(t.id)}"${rc.chosen.has(t.id) ? " checked" : ""} /> <b>${esc(t.id.toUpperCase())}</b> <span class="dim">${esc(t.description)}</span></label>`).join("");
}
function bindTypes() {
  document.querySelectorAll<HTMLInputElement>("#types input[data-type]").forEach((c) => (c.onchange = () => {
    if (c.checked) rc.chosen.add(c.dataset.type!); else rc.chosen.delete(c.dataset.type!);
  }));
}
async function loadTypes() {
  try {
    const ev = (await invoke("list_types")) as Ev;
    if (ev.event === "types") { rc.types = ev.types; rc.chosen = new Set(ev.types.map((t) => t.id)); }
  } catch (e) { rc.typesError = String(e); }
  if (rc.tab === "carve" && el("types")) { $("types").innerHTML = typesHtml(); bindTypes(); }
}

async function startCarve() {
  const source = rc.source.trim();
  if (!source) return showError("Choose a source first.");
  if (!rc.types) return showError("The list of file types has not loaded yet.");
  if (!rc.chosen.size) return showError("Tick at least one file type.");
  if (!rc.carveOut) return showError("Choose a folder to save the carved files in.");
  showError("");
  if (isDevice(source) && !(await confirmDevice("carve files"))) return;
  const part = known()?.find((p) => String(p.start_lba) === rc.carvePart);
  const { start, end } = carveRange(part);
  const types = rc.chosen.size === rc.types.length ? [] : [...rc.chosen];
  st.running = "carve"; st.mode = "";
  rc.cv = { state: "running", scanned: 0, total: 0, found: 0, rows: [], summary: "" };
  setActivity(`Carving ${source} for files...`);
  drawCarve(); syncButtons();
  try {
    await invoke("start_carve", { a: { source, out: rc.carveOut, types, start: start ?? null, end: end ?? null, map: mapUse() ?? null } });
  } catch (e) {
    st.running = ""; rc.cv.state = ""; setActivity("Idle. Nothing is running."); drawCarve(); syncButtons();
    return showError(String(e));
  }
}

function renderCarveRow(k: number, row: HTMLElement) {
  const r = rc.cv.rows[k];
  if (!r) { row.innerHTML = ""; return; }
  row.className = "vl-row frow cv";
  row.innerHTML = `<span><span class="badge" style="--c:var(--accent)">${esc(r.type.toUpperCase())}</span></span><span class="nm">${esc(r.name)}</span>
    <span class="m">${esc(fmt(r.size))}</span><span class="m dim">${fmtCount(r.offset)} B</span>
    <span>${r.complete ? `<span class="badge" style="--c:var(--green)">Complete</span>` : `<span class="badge" style="--c:var(--amber)">Incomplete</span>`}</span>`;
}

function drawCarve() {
  const prog = el<HTMLElement>("cprog");
  if (!prog) return;
  const c = rc.cv;
  $("cbar").hidden = c.state !== "running";
  bar("cbar", c.total ? (c.scanned / c.total) * 100 : 0, "Carving progress");
  prog.textContent = c.summary || (c.state === "running" ? `Scanned ${fmt(c.scanned)} of ${c.total ? fmt(c.total) : "?"}. ${fmtCount(c.found)} files found so far.` : "");
  $("cresults").hidden = c.state === "";
  if (c.state === "") return;
  if (!carveList) { carveList = new VList($("clist"), ROW_H, renderCarveRow, "Files found"); carveList.setEmpty("No files found yet."); }
  carveList.setCount(c.rows.length);
}

// ---- events from Rust ----
export function onFiles(job: Job, rows: FileRow[]) {
  if (job === "files") {
    rc.store.addMany(rows);
    if (st.screen === "recover" && rc.tab === "files") scheduleView();
  } else if (job === "carve") {
    for (const r of rows) rc.cv.rows.push({ name: r.name, type: r.type ?? "", size: r.size ?? 0, offset: r.offset ?? 0, complete: r.complete !== false });
    if (st.screen === "recover" && rc.tab === "carve") { carveList?.setCount(rc.cv.rows.length); drawCarve(); }
  }
}

export function onEvent(job: Job, ev: Ev) {
  const here = st.screen === "recover";
  if (ev.event === "error") { showError(ev.message); return; }
  if (job === "files") {
    if (ev.event === "start") { st.mode = ev.mode; st.deviceSize = ev.size; }
    else if (ev.event === "files_progress") {
      rc.recordsRead = ev.records_read; rc.recordsTotal = ev.records_total;
      setActivity(`Reading the file table: ${fmtCount(ev.records_read)} of ${fmtCount(ev.records_total)} records.`);
      if (here && rc.tab === "files") drawListing();
    } else if (ev.event === "done") {
      rc.doneInfo = `Finished. ${fmtCount(ev.files ?? rc.store.count)} files and folders, ${fmtCount(ev.deleted ?? rc.store.deletedCount)} of them deleted.`;
    }
  } else if (job === "carve") {
    if (ev.event === "start") { st.mode = ev.mode; st.deviceSize = ev.size; }
    else if (ev.event === "carve_progress") {
      rc.cv.scanned = ev.bytes_scanned; rc.cv.total = ev.bytes_total; rc.cv.found = ev.files_found;
      setActivity(`Carving: ${fmt(ev.bytes_scanned)} of ${fmt(ev.bytes_total)} scanned, ${fmtCount(ev.files_found)} files found.`);
      if (here && rc.tab === "carve") drawCarve();
    } else if (ev.event === "done") {
      rc.cv.summary = `Finished. ${fmtCount(ev.files_found ?? rc.cv.rows.length)} files found, ${fmt(ev.bytes_written ?? 0)} saved to ${rc.carveOut}.`;
    }
  } else if (job === "extract") {
    if (ev.event === "start") { st.mode = ev.mode; st.deviceSize = ev.size; }
    else if (ev.event === "extract_progress") {
      Object.assign(rc.ex, { filesDone: ev.files_done, filesTotal: ev.files_total, bytesDone: ev.bytes_done, bytesTotal: ev.bytes_total });
      setActivity(`Extracting: ${fmtCount(ev.files_done)} of ${fmtCount(ev.files_total)} files.`);
    } else if (ev.event === "extracted") {
      rc.ex.rows.push({ path: ev.path, size: ev.size, complete: ev.complete, note: ev.note });
    } else if (ev.event === "done") {
      const bad = rc.ex.rows.filter((r) => !r.complete).length;
      rc.ex.summary = `Finished. ${fmtCount(ev.extracted ?? rc.ex.rows.length)} extracted${ev.failed ? `, ${fmtCount(ev.failed)} failed` : ""}${bad ? `, ${fmtCount(bad)} incomplete (hover a row or read its note)` : ""}. Saved to ${rc.out}.`;
      rc.ex.filesDone = rc.ex.filesTotal = ev.extracted ?? rc.ex.filesTotal;
    }
    if (here && rc.tab === "files") drawExtract();
  }
}

export function onExit(x: Exit) {
  const state = x.code === 0 ? "done" : x.code === null ? "stopped" : "error";
  if (x.job === "files") {
    rc.listing = state;
    setActivity(state === "done" ? "Idle. File list ready." : state === "stopped" ? "File listing stopped." : "File listing stopped by an error.");
    if (st.screen === "recover" && rc.tab === "files") drawListing();
  } else if (x.job === "carve") {
    rc.cv.state = state;
    if (state === "stopped") rc.cv.summary = `Stopped. ${fmtCount(rc.cv.rows.length)} files were saved before stopping; the last one may be incomplete.`;
    if (state === "error" && !rc.cv.summary) rc.cv.summary = "Carving stopped by an error.";
    setActivity(state === "done" ? "Idle. Carving finished." : state === "stopped" ? "Carving stopped." : "Carving stopped by an error.");
    if (st.screen === "recover" && rc.tab === "carve") drawCarve();
  } else if (x.job === "extract") {
    rc.ex.state = state;
    if (state === "stopped") rc.ex.summary = `Stopped. ${fmtCount(rc.ex.rows.length)} files were written; the last one may be incomplete.`;
    if (state === "error" && !rc.ex.summary) rc.ex.summary = "Extraction stopped by an error.";
    setActivity(state === "done" ? "Idle. Extraction finished." : state === "stopped" ? "Extraction stopped." : "Extraction stopped by an error.");
    if (st.screen === "recover" && rc.tab === "files") drawExtract();
  }
  syncButtons();
}
