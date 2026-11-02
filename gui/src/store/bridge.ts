import { listen } from "@tauri-apps/api/event";
import type { Ev, Exit, FileRow, Job } from "../lib/contract";
import { onEvent, onExit, onFiles } from "./events";

/**
 * Subscribes to the Rust side's events. Called once at startup, outside React,
 * so StrictMode's double-run effects can't register listeners twice.
 */
export async function startBridge(): Promise<void> {
  await Promise.all([
    listen<{ job: Job; event: Ev }>("sf-event", (e) => onEvent(e.payload.job, e.payload.event)),
    listen<{ job: Job; files: FileRow[] }>("sf-files", (e) => onFiles(e.payload.job, e.payload.files)),
    listen<string>("sf-log", (e) => console.warn("sf-cli:", e.payload)),
    listen<Exit>("sf-exit", (e) => onExit(e.payload)),
  ]);
}
