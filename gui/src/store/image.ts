import { create } from "zustand";
import type { Totals } from "../lib/contract";
import { fmtBytes } from "../lib/format";
import type { Block } from "../lib/map";

export interface Tiles {
  rescued: string;
  bad: string;
  pending: string;
}

export const NO_TILES: Tiles = { rescued: "–", bad: "–", pending: "–" };

export interface ImageState {
  /** Latest map snapshot, for the health map. */
  snapshot: { size: number; blocks: Block[] } | null;
  hasMap: boolean;
  mapDone: boolean;
  tiles: Tiles;
  pass: string;
  chunkKiB: number;
  skipKiB: number;
  retries: number;
  /** Retry bad areas only (pass 2), for a map that has already been through pass 1. */
  retryOnly: boolean;
}

export const useImage = create<ImageState>(() => ({
  snapshot: null,
  hasMap: false,
  mapDone: false,
  tiles: NO_TILES,
  retryOnly: false,
  pass: "–",
  chunkKiB: 1024,
  skipKiB: 1024,
  retries: 3,
}));

export function tilesFromTotals(size: number, t: Totals): Tiles {
  return {
    rescued: size ? `${((t.finished / size) * 100).toFixed(2)}%` : "–",
    bad: fmtBytes(t.bad_sector),
    pending: fmtBytes(Math.max(0, size - t.finished - t.bad_sector)),
  };
}

export function tilesFromProgress(rescued: number, total: number, bad: number, pending: number): Tiles {
  return {
    rescued: total ? `${((rescued / total) * 100).toFixed(2)}%` : "–",
    bad: fmtBytes(bad),
    pending: fmtBytes(pending),
  };
}
