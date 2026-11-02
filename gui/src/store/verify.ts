import { create } from "zustand";

export interface HashResult {
  source: string;
  size: number;
  sha256: string;
  at: Date;
}

export interface VerifyState {
  source: string;
  /** Source of the run in progress. */
  hashing: string;
  size: number;
  done: number;
  /** Newest first, so two runs can be compared at a glance. */
  results: HashResult[];
}

export const useVerify = create<VerifyState>(() => ({ source: "", hashing: "", size: 0, done: 0, results: [] }));
