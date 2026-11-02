// Shared UI state and the small helpers every screen uses.
import { invoke } from "@tauri-apps/api/core";
import type { CliInfo, Device, Job, Partition } from "./contract.ts";
import { $, isDevice } from "./util.ts";

export type Screen = "image" | "partitions" | "recover";

export const st = {
  screen: "image" as Screen,
  running: "" as "" | Job,
  activity: "Idle. Nothing is running.",
  mode: "" as "" | "device" | "file", // from the CLI's start event; falls back to the source field
  hasMap: false, mapDone: false,
  source: "", dest: "", map: "",
  deviceSize: 0, sectorSize: 512,
  pass: "", pollTimer: 0 as number,
  /** Scan result; `source` is what was scanned, so other screens know whether it applies to them. */
  partitions: null as null | { kind: string; list: Partition[]; source: string },
  scanSource: "",
  error: "",
  cli: null as CliInfo | null,
  devices: [] as Device[],
  /** null = unknown (devices not listed yet). */
  elevated: null as boolean | null,
  devicesLoaded: false, devicesLoading: false, devicesError: "",
  /** Lower-cased source path for which the user ticked "image the system drive anyway". */
  systemConfirmed: "",
};

const MODE_TEXT = {
  device: ["Direct hardware access", "SectorForge reads the physical drive itself. It needs Administrator rights, and it only ever reads from the drive. Reading a failing drive stresses it, so the first pass skips damage quickly."],
  file: ["Image file", "SectorForge is working on a file, not the drive, so the original hardware is not touched and the file can be re-read as often as needed."],
} as const;

export function renderStatusBar() {
  $("act").textContent = st.activity;
  const m = st.mode || (st.source ? (isDevice(st.source) ? "device" : "file") : "");
  $("mode").innerHTML = m ? `Mode: <b>${MODE_TEXT[m][0]}</b>. ${MODE_TEXT[m][1]}` : "Mode: choose a source to see how SectorForge will access it.";
}
export function setActivity(t: string) { st.activity = t; renderStatusBar(); }
export function showError(msg: string) { st.error = msg; const e = document.getElementById("err"); if (e) { e.textContent = msg; e.hidden = !msg; } }

/** Start a partition scan; the result lands in `st.partitions`. Throws a readable message on failure. */
export async function beginScan(source: string, deep: boolean) {
  // State first: events can arrive before the invoke call returns.
  st.scanSource = source; st.partitions = null;
  st.running = "scan"; st.mode = "";
  setActivity(`Reading the partition table of ${source}...`);
  try { await invoke("start_scan", { source, deep }); }
  catch (e) { st.running = ""; setActivity("Idle. Nothing is running."); throw e; }
}

/** Load (or reload) the drive list. Never throws; failures are shown in the picker. */
export async function loadDevices(onDone: () => void) {
  if (st.devicesLoading) return;
  st.devicesLoading = true; st.devicesError = ""; onDone();
  try {
    const ev = (await invoke("list_devices")) as { event: "devices"; elevated: boolean; devices: Device[] };
    st.devices = ev.devices; st.elevated = ev.elevated; st.devicesLoaded = true;
  } catch (e) {
    st.devicesError = String(e); st.devices = [];
  }
  st.devicesLoading = false; onDone();
}
