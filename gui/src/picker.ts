// Drive picker (from `sf-cli devices`) shared by the Image and Partitions screens.
import { invoke } from "@tauri-apps/api/core";
import { busLabel, deviceTitle, findDevice } from "./devices.ts";
import { loadDevices, st } from "./state.ts";
import { esc, fmt, isDevice } from "./util.ts";

let host: HTMLElement | null = null;
let onPick: (path: string) => void = () => {};
let elevMsg = "";

export function mountPicker(el: HTMLElement, pick: (path: string) => void) {
  host = el; onPick = pick;
  drawPicker();
  if (!st.devicesLoaded) loadDevices(drawPicker);
}

/** Redraw after the source field changed (selection highlight, elevation notice). */
export function refreshPicker() { if (host?.isConnected) drawPicker(); }

function platform(): string {
  return st.cli?.platform ?? (navigator.userAgent.includes("Windows") ? "windows" : "linux");
}

/** Explains what elevated access means on this OS; shown when a drive is chosen and we lack it. */
function elevationNotice(): string {
  if (!isDevice(st.source) || st.elevated !== false) return "";
  if (platform() === "windows") {
    return `<div class="notice warn"><p><b>Administrator rights needed.</b> Windows only lets Administrators read a drive directly. SectorForge will close and reopen as Administrator with these settings kept. Windows will ask you to confirm.</p>
      <div class="actions"><button class="btn primary" id="elevate">Restart as Administrator</button><span class="dim" id="elev-msg">${esc(elevMsg)}</span></div></div>`;
  }
  return st.cli?.pkexec === false
    ? `<div class="notice warn"><p><b>Administrator rights needed, and pkexec was not found.</b> Reading a drive directly needs root. Install polkit (the pkexec command), start SectorForge with sudo, or add your user to the disk group. Or work from an image file instead.</p></div>`
    : `<div class="notice"><p><b>Administrator rights needed.</b> When you start, your system will ask for your password (through pkexec) so SectorForge can read the drive.</p></div>`;
}

function drawPicker() {
  if (!host) return;
  const sel = findDevice(st.devices, st.source)?.path;
  let body: string;
  if (st.devicesLoading && !st.devicesLoaded) body = `<p class="dim">Looking for drives...</p>`;
  else if (st.devicesError) body = `<p class="dim">Could not list drives: ${esc(st.devicesError)}</p>`;
  else if (!st.devices.length) body = `<p class="dim">No drives were found. You can still type an image file path below.</p>`;
  else {
    body = `<div class="devs" role="radiogroup" aria-label="Drives found">${st.devices.map((d) => `
      <button class="dev" role="radio" aria-checked="${d.path === sel}" data-path="${esc(d.path)}">
        <span class="dt">${esc(deviceTitle(d))}${d.system ? ` <span class="badge" style="--c:var(--red)">System drive</span>` : ""}${d.removable ? ` <span class="badge" style="--c:var(--dim)">Removable</span>` : ""}</span>
        <span class="dm">${esc(fmt(d.size))}, ${esc(busLabel(d.bus))}</span>
        <span class="dp mono">${esc(d.path)}</span>
      </button>`).join("")}</div>`;
  }
  host.innerHTML = `<section class="panel" aria-label="Drives">
    <div class="panel-head"><h3>Drives</h3><button class="btn small" id="dev-refresh"${st.devicesLoading ? " disabled" : ""}>Refresh</button></div>
    ${body}${elevationNotice()}</section>`;
  host.querySelectorAll<HTMLButtonElement>(".dev").forEach((b) => (b.onclick = () => { onPick(b.dataset.path!); elevMsg = ""; drawPicker(); }));
  host.querySelector<HTMLButtonElement>("#dev-refresh")!.onclick = () => loadDevices(drawPicker);
  const el = host.querySelector<HTMLButtonElement>("#elevate");
  if (el) el.onclick = async () => {
    try {
      await invoke("restart_as_admin", { restore: { source: st.source, dest: st.dest, map: st.map, screen: st.screen } });
    } catch (e) { elevMsg = String(e); drawPicker(); }
  };
}
