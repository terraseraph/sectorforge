// Typed wrappers for the Tauri commands in src-tauri/src/lib.rs.
import { showDialog } from "../store/dialog";
import { pickPath } from "../store/picker";
import type { CliInfo, Ev, Restore } from "./contract";
import { invoke } from "./ipc";

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
  access: Access;
  /** Seconds per pass-through read (SCSI/ATA only). */
  timeout: number;
  /** Run pass 1 from the end of the drive backwards. */
  reverse: boolean;
}
/** How sf-cli reads a physical drive (`--access`). */
export type Access = "os" | "scsi" | "ata";
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

/** A drive letter (or / and home on Linux) the in-app file picker starts from. */
export interface Place { path: string; label: string; free: number | null; total: number | null }
export interface DirEntry { name: string; dir: boolean; size: number }

export const api = {
  cliInfo: () => invoke<CliInfo>("cli_info"),
  listDevices: () => invoke<EvOf<"devices">>("list_devices"),
  listTypes: () => invoke<EvOf<"types">>("list_types"),
  getStatus: (map: string) => invoke<EvOf<"status">>("get_status", { map }),
  startupRestore: () => invoke<Restore | null>("startup_restore"),
  restartAsAdmin: (restore: Restore) => invoke<void>("restart_as_admin", { restore }),
  listPlaces: () => invoke<Place[]>("list_places"),
  listDir: (path: string) => invoke<DirEntry[]>("list_dir", { path }),
  createFolder: (path: string) => invoke<void>("create_folder", { path }),

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

// The app's own picker and dialogs, the same in the window and in a browser tab (which can't open
// native pickers, and where some webviews block prompt()/confirm() and answer "Cancel" unseen).
export const dialogs = {
  openFile: (title: string) => pickPath({ title, mode: "open" }),
  openFolder: (title: string) => pickPath({ title, mode: "folder" }),
  /** `need`: bytes the file will take, so drives without room are flagged. */
  saveFile: (title: string, initial?: string, need?: number) => pickPath({ title, mode: "save", initial, need }),
  confirmReadDrive: async (what: string) =>
    (await showDialog({
      title: "Read the drive directly?",
      message: `You are about to ${what} by reading the live drive directly. That is fine for a healthy drive, but reading a failing drive repeatedly can make it worse.

If the drive is failing, image it first (Image screen) and recover from the image.`,
      okLabel: "Read the drive",
    })) !== null,
};
