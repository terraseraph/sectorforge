// Everything the UI can ask for. Rule for every job: set the "running" state
// *before* invoking, because sf-cli events can arrive before invoke() resolves,
// and roll it back if the invoke is rejected.
import { api, dialogs, errorText } from "../lib/api";
import type { Job } from "../lib/contract";
import { fmtCount, isDevicePath } from "../lib/format";
import { carveRange, mapForSource } from "../lib/recoverlogic";
import { IDLE, type Screen, systemDriveBlocked, useApp } from "./app";
import { NO_TILES, tilesFromTotals, useImage } from "./image";
import { NO_CARVE, NO_EXTRACT, NO_FILTER, bumpSelection, files, selection, useRecover } from "./recover";
import { partitionsFor, useScan } from "./scan";

const app = useApp.getState;

export const setActivity = (activity: string) => useApp.setState({ activity });
export const showError = (error: string) => useApp.setState({ error });
export const goTo = (screen: Screen) => useApp.setState({ screen, error: "" });

/** Marks `job` as running; returns false (and shows why) if something else already is. */
function begin(job: Job, activity: string): boolean {
  const running = app().running;
  if (running) {
    showError(`Wait for the current ${running} job to finish, or stop it first.`);
    return false;
  }
  useApp.setState({ running: job, jobMode: "", activity, error: "" });
  return true;
}

function rollback(e: unknown) {
  useApp.setState({ running: null, activity: IDLE, error: errorText(e) });
}

// ---- launch ----

export async function checkCli() {
  try {
    useApp.setState({ cli: await api.cliInfo() });
  } catch (e) {
    useApp.setState({
      cli: { path: "", searched: [], found: false, version: null, contract: null, problem: `Could not check sf-cli: ${errorText(e)}`, platform: "", pkexec: true },
    });
  }
}

/** After "Restart as Administrator", the new process gets the old settings back. */
export async function restoreAfterElevation() {
  try {
    const r = await api.startupRestore();
    if (!r) return;
    const screen: Screen = r.screen === "partitions" || r.screen === "recover" ? r.screen : "image";
    useApp.setState({ source: r.source, dest: r.dest, map: r.map, screen });
  } catch {
    // Not fatal: the user just re-enters their paths.
  }
}

export async function loadDevices() {
  if (app().devicesState === "loading") return;
  useApp.setState({ devicesState: "loading", devicesError: "" });
  try {
    const ev = await api.listDevices();
    useApp.setState({ devices: ev.devices, elevated: ev.elevated, devicesState: "loaded" });
  } catch (e) {
    useApp.setState({ devices: [], devicesState: "error", devicesError: errorText(e) });
  }
}

/** Returns an error message to show next to the button, or null if the app is restarting. */
export async function restartAsAdmin(): Promise<string | null> {
  const { source, dest, map, screen } = app();
  try {
    await api.restartAsAdmin({ source, dest, map, screen });
    return null;
  } catch (e) {
    return errorText(e);
  }
}

// ---- Image ----

let pollTimer: ReturnType<typeof setInterval> | undefined;

export function stopPolling() {
  clearInterval(pollTimer);
  pollTimer = undefined;
}

/** Reads the map. `quiet`: a missing map is normal before the first run, so don't show it. */
export async function pollStatus(quiet = true) {
  const map = app().map.trim();
  if (!map) return;
  try {
    const ev = await api.getStatus(map);
    const live = app().running === "image";
    useImage.setState((s) => ({
      snapshot: { size: ev.size, blocks: ev.blocks },
      hasMap: true,
      mapDone: ev.size > 0 && ev.totals.finished >= ev.size,
      tiles: live ? s.tiles : tilesFromTotals(ev.size, ev.totals),
    }));
  } catch (e) {
    if (!quiet) showError(errorText(e));
  }
}

/** After the map path was edited: the old snapshot no longer applies, so drop it and read the new map. */
export function mapChanged() {
  if (app().running === "image") return;
  useImage.setState({ snapshot: null, hasMap: false, mapDone: false, tiles: NO_TILES, pass: "–" });
  void pollStatus();
}

const positive = (n: number, fallback: number) => (Number.isFinite(n) && n >= 1 ? Math.floor(n) : fallback);

export async function startImage() {
  const { source, dest, map } = app();
  if (!source.trim() || !dest.trim() || !map.trim()) return showError("Choose a source, a destination and a map file first.");
  if (systemDriveBlocked(app())) {
    return showError("That is your system drive. Tick the box under the Source field if you really mean to image it.");
  }
  const { chunkKiB, skipKiB, retries } = useImage.getState();
  if (!begin("image", `Starting: opening ${source.trim()}...`)) return;
  useImage.setState({ pass: "–" });
  try {
    await api.startImage({
      source: source.trim(),
      dest: dest.trim(),
      map: map.trim(),
      chunkSize: positive(chunkKiB, 1024) * 1024,
      skipSize: positive(skipKiB, 1024) * 1024,
      retries: Number.isFinite(retries) && retries >= 0 ? Math.floor(retries) : 3,
    });
  } catch (e) {
    return rollback(e);
  }
  // The job may already have ended (e.g. refused at once); only poll a live run.
  if (app().running === "image") {
    stopPolling();
    pollTimer = setInterval(() => void pollStatus(), 1000);
  }
}

export function pauseImage() {
  void api.stopImage();
  setActivity("Pausing. The map is saved, so Resume picks up from here.");
}

// ---- Partitions ----

export async function startScan(source: string, deep: boolean) {
  const src = source.trim();
  if (!src) return showError("Enter a source first.");
  if (!begin("scan", `Reading the partition table of ${src}...`)) return;
  useScan.setState({ scanning: src, pct: 0, result: null, deviceSize: 0 });
  try {
    await api.startScan(src, deep);
  } catch (e) {
    rollback(e);
  }
}

export const cancelScan = () => void api.stopScan();

// ---- Recover ----

export function recoverSource(): string {
  const r = useRecover.getState();
  return (r.sourceEdited ? r.source : app().dest).trim();
}

/** The imaging map, when the recovery source is the image it describes. */
export function recoverMap(source: string): string | null {
  const { dest, map } = app();
  return mapForSource(source, dest.trim(), map.trim(), useImage.getState().hasMap) ?? null;
}

async function okToReadDevice(source: string, what: string): Promise<boolean> {
  return !isDevicePath(source) || (await dialogs.confirmReadDrive(what));
}

export async function startListing() {
  const source = recoverSource();
  const { lba, deletedOnlyRun, listing, listedFor } = useRecover.getState();
  if (!source) return showError("Choose a source first.");
  if (lba === null) return showError("Choose an NTFS partition first. If the list is empty, press Scan source.");
  if (!(await okToReadDevice(source, "list files"))) return;
  if (!begin("files", `Reading the file table of ${source}...`)) return;

  const before = { listing, listedFor };
  files.clear();
  selection.clear();
  useRecover.setState((s) => ({
    listing: "running",
    listedFor: { source, lba },
    recordsRead: 0,
    recordsTotal: 0,
    doneInfo: "",
    folder: "/",
    filter: NO_FILTER,
    extract: NO_EXTRACT,
    filesVersion: s.filesVersion + 1,
    selVersion: s.selVersion + 1,
  }));
  try {
    await api.startFiles({ source, partitionLba: lba, deletedOnly: deletedOnlyRun, map: recoverMap(source) });
  } catch (e) {
    useRecover.setState({ listing: before.listing === "running" ? "" : before.listing, listedFor: before.listedFor });
    rollback(e);
  }
}

export const stopListing = () => void api.stopFiles();

export async function startExtract() {
  const { listedFor, out } = useRecover.getState();
  if (!listedFor) return;
  const ids = [...selection].map((i) => files.ids[i]).filter((id): id is number => id !== undefined && id >= 0);
  if (!ids.length) return showError("Select at least one file or folder first.");
  if (!out.trim()) return showError("Choose a folder to extract into.");
  const { source, lba } = listedFor;
  if (!(await okToReadDevice(source, "extract files"))) return;
  if (!begin("extract", `Extracting ${fmtCount(ids.length)} selected items to ${out.trim()}...`)) return;
  useRecover.setState({ extract: { ...NO_EXTRACT, state: "running", filesTotal: ids.length } });
  try {
    await api.startExtract({ source, partitionLba: lba, ids, out: out.trim(), map: recoverMap(source) });
  } catch (e) {
    useRecover.setState({ extract: NO_EXTRACT });
    rollback(e);
  }
}

export const stopExtract = () => void api.stopExtract();

export async function loadTypes() {
  try {
    const ev = await api.listTypes();
    useRecover.setState({ types: ev.types, chosen: ev.types.map((t) => t.id), typesError: "" });
  } catch (e) {
    useRecover.setState({ typesError: errorText(e) });
  }
}

export async function startCarve() {
  const source = recoverSource();
  const { types, chosen, carveOut, carvePart } = useRecover.getState();
  if (!source) return showError("Choose a source first.");
  if (!types) return showError("The list of file types has not loaded yet.");
  if (!chosen.length) return showError("Tick at least one file type.");
  if (!carveOut.trim()) return showError("Choose a folder to save the carved files in.");
  if (!(await okToReadDevice(source, "carve files"))) return;

  const part = partitionsFor(useScan.getState().result, source)?.find((p) => String(p.start_lba) === carvePart);
  const { start, end } = carveRange(part);
  if (!begin("carve", `Carving ${source} for files...`)) return;
  useRecover.setState({ carve: { ...NO_CARVE, state: "running" } });
  try {
    await api.startCarve({
      source,
      out: carveOut.trim(),
      types: chosen.length === types.length ? [] : chosen,
      start: start ?? null,
      end: end ?? null,
      map: recoverMap(source),
    });
  } catch (e) {
    useRecover.setState({ carve: NO_CARVE });
    rollback(e);
  }
}

export const stopCarve = () => void api.stopCarve();

// ---- selection ----

export function setSelected(indexes: Iterable<number>, on: boolean) {
  for (const i of indexes) {
    if (on) selection.add(i);
    else selection.delete(i);
  }
  bumpSelection();
}

export function clearSelection() {
  selection.clear();
  bumpSelection();
}
