import { create } from "zustand";
import type { CliInfo, Device, Job } from "../lib/contract";
import { systemDriveFor } from "../lib/devices";

export type Screen = "image" | "partitions" | "recover";
export type LoadState = "idle" | "loading" | "loaded" | "error";

export const IDLE = "Idle. Nothing is running.";

export interface AppState {
  screen: Screen;
  /** The one job running right now (the UI never runs two at once). */
  running: Job | null;
  activity: string;
  /** The current error, shown at the top of the active screen. */
  error: string;
  /** Access mode from the current job's `start` event; "" = infer it from the source path. */
  jobMode: "" | "device" | "file";
  cli: CliInfo | null;

  // Shared between screens: the Image destination is the default Recover source.
  source: string;
  dest: string;
  map: string;
  /** Lower-cased source path the user confirmed imaging even though it is the system drive. */
  systemConfirmed: string;

  devices: Device[];
  /** null = not known yet (drives not listed). */
  elevated: boolean | null;
  devicesState: LoadState;
  devicesError: string;
}

export const useApp = create<AppState>(() => ({
  screen: "image",
  running: null,
  activity: IDLE,
  error: "",
  jobMode: "",
  cli: null,
  source: "",
  dest: "",
  map: "",
  systemConfirmed: "",
  devices: [],
  elevated: null,
  devicesState: "idle",
  devicesError: "",
}));

const norm = (p: string) => p.trim().toLowerCase();

/** The source is the system drive and the user hasn't explicitly confirmed imaging it. */
export function systemDriveBlocked(s: Pick<AppState, "devices" | "source" | "systemConfirmed">): boolean {
  return !!systemDriveFor(s.devices, s.source) && s.systemConfirmed !== norm(s.source);
}

export const confirmKey = norm;
