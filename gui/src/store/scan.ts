import { create } from "zustand";
import type { Partition } from "../lib/contract";

export interface ScanResult {
  /** "mbr" | "gpt" | "none" */
  kind: string;
  partitions: Partition[];
  /** What was scanned, so Recover only offers partitions that belong to its source. */
  source: string;
  deviceSize: number;
}

export interface ScanState {
  deep: boolean;
  /** Source of the scan in progress. */
  scanning: string;
  deviceSize: number;
  /** 0..100 during a signature scan; 0 otherwise. */
  pct: number;
  result: ScanResult | null;
  /** Every scan this session, by lower-cased source, so Home can show each drive's layout. */
  bySource: Record<string, ScanResult>;
  /** The last scan error per lower-cased source: a read error here often means a failing drive. */
  errors: Record<string, string>;
  /** Lower-cased sources the startup survey is still reading. */
  surveying: string[];
}

export const useScan = create<ScanState>(() => ({
  deep: false,
  scanning: "",
  deviceSize: 0,
  pct: 0,
  result: null,
  bySource: {},
  errors: {},
  surveying: [],
}));

const same = (a: string, b: string) => a.trim().toLowerCase() === b.trim().toLowerCase();

/** Partitions known for `source`, or null if it hasn't been scanned. */
export function partitionsFor(result: ScanResult | null, source: string): Partition[] | null {
  return result && source.trim() && same(result.source, source) ? result.partitions : null;
}
