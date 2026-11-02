import { create } from "zustand";
import type { Access } from "../lib/api";
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
  chunkBytes: number;
  skipBytes: number;
  retries: number;
  /** Retry bad areas only (pass 2), for a map that has already been through pass 1. */
  retryOnly: boolean;
  access: Access;
  timeoutSecs: number;
  reverse: boolean;
}

const KiB = 1024;
const MiB = 1024 * KiB;

export const useImage = create<ImageState>(() => ({
  snapshot: null,
  hasMap: false,
  mapDone: false,
  tiles: NO_TILES,
  retryOnly: false,
  pass: "–",
  chunkBytes: MiB,
  skipBytes: MiB,
  retries: 3,
  access: "os",
  timeoutSecs: 10,
  reverse: false,
}));

export type PresetSettings = Pick<ImageState, "chunkBytes" | "skipBytes" | "retries" | "timeoutSecs">;

/** One-click Advanced settings. Access mode and direction are left alone: they depend on the drive. */
export const PRESETS: { id: string; label: string; hint: string; s: PresetSettings }[] = [
  {
    id: "healthy",
    label: "Healthy drive",
    hint: "Big reads and one retry. Fastest when the drive reads cleanly.",
    s: { chunkBytes: 4 * MiB, skipBytes: 4 * MiB, retries: 1, timeoutSecs: 10 },
  },
  {
    id: "failing",
    label: "Some bad sectors",
    hint: "The default. Balanced for a drive that is starting to fail.",
    s: { chunkBytes: MiB, skipBytes: MiB, retries: 3, timeoutSecs: 10 },
  },
  {
    id: "damaged",
    label: "Badly damaged",
    hint: "Small reads, big jumps past damage and one retry, to get the readable parts off before the drive gets worse.",
    s: { chunkBytes: 64 * KiB, skipBytes: 16 * MiB, retries: 1, timeoutSecs: 5 },
  },
  {
    id: "thorough",
    label: "Thorough",
    hint: "Small reads and many retries per sector. Slowest; for squeezing out the last sectors on a retry run.",
    s: { chunkBytes: 128 * KiB, skipBytes: MiB, retries: 8, timeoutSecs: 30 },
  },
];

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
