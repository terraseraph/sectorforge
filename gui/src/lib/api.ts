// Typed wrappers for the Tauri commands in src-tauri/src/lib.rs.
import { open, save } from "@tauri-apps/plugin-dialog";
import { showDialog } from "../store/dialog";
import type { CliInfo, Ev, Restore } from "./contract";
import { inTauri, invoke } from "./ipc";

export type EvOf<K extends Ev["event"]> = Extract<Ev, { event: K }>;

export interface ImageArgs {
  source: string;
  dest: string;
  map: string;
  chunkSize: number;
  skipSize: number;
  retries: number;
  /** Only pass 2: go back over the areas a previous run could not read. */
  retryOnly: boolean;
}
export interface FilesArgs { source: string; partitionLba: number; deletedOnly: boolean; map: string | null }
export interface ExtractArgs { source: string; partitionLba: number; ids: number[]; out: string; map: string | null }
export interface CarveArgs {
  source: string;
  out: string;
  /** Empty = every type. */
  types: string[];
  start: number | null;
  end: number | null;
  map: string | null;
}

export const api = {
  cliInfo: () => invoke<CliInfo>("cli_info"),
  listDevices: () => invoke<EvOf<"devices">>("list_devices"),
  listTypes: () => invoke<EvOf<"types">>("list_types"),
  getStatus: (map: string) => invoke<EvOf<"status">>("get_status", { map }),
  startupRestore: () => invoke<Restore | null>("startup_restore"),
  restartAsAdmin: (restore: Restore) => invoke<void>("restart_as_admin", { restore }),

  startImage: (a: ImageArgs) => invoke<void>("start_image", { a }),
  stopImage: () => invoke<void>("stop_image"),
  startScan: (source: string, deep: boolean) => invoke<void>("start_scan", { source, deep }),
  stopScan: () => invoke<void>("stop_scan"),
  startHash: (source: string) => invoke<void>("start_hash", { source }),
  stopHash: () => invoke<void>("stop_hash"),
  startSurvey: (sources: string[]) => invoke<void>("start_survey", { sources }),
  startFiles: (a: FilesArgs) => invoke<void>("start_files", { a }),
  stopFiles: () => invoke<void>("stop_files"),
  startExtract: (a: ExtractArgs) => invoke<void>("start_extract", { a }),
  stopExtract: () => invoke<void>("stop_extract"),
  startCarve: (a: CarveArgs) => invoke<void>("start_carve", { a }),
  stopCarve: () => invoke<void>("stop_carve"),
};

/** Commands reject with a plain-English string; anything else is unexpected but still shown. */
export function errorText(e: unknown): string {
  if (typeof e === "string") return e;
  if (e instanceof Error) return e.message;
  return String(e);
}

const single = (v: string | string[] | null) => (typeof v === "string" ? v : null);

// A browser tab can't open the native pickers, so it asks for the path as text, in the app's own dialog
// (never prompt()/confirm(): some webviews block them and answer "Cancel" without showing anything).
const askPath = (title: string, input = "") => showDialog({ title, input, okLabel: "OK" });

export const dialogs = {
  openFile: async (title: string) => (inTauri ? single(await open({ title })) : askPath(title)),
  openFolder: async (title: string) => (inTauri ? single(await open({ title, directory: true })) : askPath(title)),
  saveFile: async (title: string, defaultPath?: string) => (inTauri ? single(await save({ title, defaultPath })) : askPath(title, defaultPath)),
  confirmReadDrive: async (what: string) =>
    (await showDialog({
      title: "Read the drive directly?",
      message: `You are about to ${what} by reading the live drive directly. That is fine for a healthy drive, but reading a failing drive repeatedly can make it worse.

If the drive is failing, image it first (Image screen) and recover from the image.`,
      okLabel: "Read the drive",
    })) !== null,
};
