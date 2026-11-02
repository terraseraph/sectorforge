import { create } from "zustand";
import type { CarveType } from "../lib/contract";
import { FileStore, type Filter } from "../lib/filestore";

export type RunState = "" | "running" | "done" | "stopped" | "error";

export interface ExtractedRow { path: string | null; size: number; complete: boolean; note: string | null }
export interface CarvedRow { name: string; type: string; size: number; offset: number; complete: boolean }

export interface ExtractState {
  state: RunState;
  filesDone: number;
  filesTotal: number;
  bytesDone: number;
  bytesTotal: number;
  rows: ExtractedRow[];
  summary: string;
}

export interface CarveState {
  state: RunState;
  scanned: number;
  total: number;
  found: number;
  rows: CarvedRow[];
  summary: string;
}

export interface RecoverState {
  tab: "files" | "carve";
  /** Until edited, the source follows the Image screen's destination. */
  source: string;
  sourceEdited: boolean;

  lba: number | null;
  deletedOnlyRun: boolean;
  listing: RunState;
  /** What `files` currently holds; extraction must use exactly this source and partition. */
  listedFor: { source: string; lba: number } | null;
  recordsRead: number;
  recordsTotal: number;
  doneInfo: string;
  folder: string;
  filter: Filter;
  /** Bumped when `files` changes (it is mutable and too big to copy). */
  filesVersion: number;
  /** Bumped when `selection` changes. */
  selVersion: number;
  out: string;
  extract: ExtractState;

  types: CarveType[] | null;
  typesError: string;
  chosen: string[];
  carvePart: string;
  carveOut: string;
  carve: CarveState;
}

export const NO_EXTRACT: ExtractState = { state: "", filesDone: 0, filesTotal: 0, bytesDone: 0, bytesTotal: 0, rows: [], summary: "" };
export const NO_CARVE: CarveState = { state: "", scanned: 0, total: 0, found: 0, rows: [], summary: "" };
export const NO_FILTER: Filter = { deletedOnly: false, recoverableOnly: false, text: "" };

export const useRecover = create<RecoverState>(() => ({
  tab: "files",
  source: "",
  sourceEdited: false,
  lba: null,
  deletedOnlyRun: false,
  listing: "",
  listedFor: null,
  recordsRead: 0,
  recordsTotal: 0,
  doneInfo: "",
  folder: "/",
  filter: NO_FILTER,
  filesVersion: 0,
  selVersion: 0,
  out: "",
  extract: NO_EXTRACT,
  types: null,
  typesError: "",
  chosen: [],
  carvePart: "",
  carveOut: "",
  carve: NO_CARVE,
}));

/**
 * The listing can hold a million+ entries, so it lives outside React state in a
 * column store; components re-read it when `filesVersion` changes.
 */
export const files = new FileStore();
/** Selected entries, as indexes into `files`. */
export const selection = new Set<number>();

export const bumpSelection = () => useRecover.setState((s) => ({ selVersion: s.selVersion + 1 }));
