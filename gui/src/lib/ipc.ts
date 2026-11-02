// Inside the Tauri window: Tauri's own IPC. In a plain browser tab (`npm run dev`
// running, page at http://localhost:<vite port>) the same commands and events go to the running
// app over its debug-only bridge (src-tauri/src/devbridge.rs). Real data either way.
import { invoke as tauriInvoke } from "@tauri-apps/api/core";
import { listen as tauriListen } from "@tauri-apps/api/event";

export const inTauri = "__TAURI_INTERNALS__" in window;
// scripts/dev.mjs sets the port it picked; 1430 when Vite is started some other way.
const BRIDGE = `http://127.0.0.1:${import.meta.env.VITE_SF_BRIDGE_PORT ?? "1430"}`;

export async function invoke<T>(cmd: string, args: Record<string, unknown> = {}): Promise<T> {
  if (inTauri) return tauriInvoke<T>(cmd, args);
  let res: Response;
  try {
    res = await fetch(`${BRIDGE}/invoke`, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ cmd, args }) });
  } catch {
    throw "SectorForge isn't running. Start it with `npm run dev` and this tab will talk to it.";
  }
  const body: unknown = await res.json();
  // Rejections are plain-English strings, the same as from a Tauri command.
  if (!res.ok) throw body;
  return body as T;
}

let events: EventSource | null = null;

export async function listen<T>(name: string, cb: (e: { payload: T }) => void): Promise<void> {
  if (inTauri) {
    await tauriListen<T>(name, cb);
    return;
  }
  events ??= new EventSource(`${BRIDGE}/events`);
  events.addEventListener(name, (e) => cb({ payload: JSON.parse((e as MessageEvent<string>).data) as T }));
}
