// Types for the sf-cli NDJSON contract (docs/cli-contract.md, v2).
import type { Block } from "./map";

export interface Totals { finished: number; bad_sector: number; non_trimmed: number; non_scraped: number; non_tried: number }
export interface Partition { start_lba: number; size_sectors: number | null; filesystem: string; confidence: string; source: string; label: string | null }
export interface Device {
  path: string; model: string | null; serial: string | null; size: number | null; sector_size: number | null;
  bus: string; removable: boolean; system: boolean;
}
export interface CarveType { id: string; description: string }
export type Recoverable = "full" | "partial" | "none";

/** One `file` event from `files` or `carve` (the Rust side batches them; absent fields = not applicable). */
export interface FileRow {
  id?: number; parent_id?: number; name: string; path?: string | null; is_dir?: boolean; size?: number;
  deleted?: boolean; modified?: string | null; created?: string | null; recoverable?: Recoverable;
  offset?: number; type?: string; complete?: boolean;
}

export type Ev =
  | { event: "start"; source: string; mode: "device" | "file"; size: number; sector_size: number; resumed: boolean }
  | { event: "pass_start"; pass: number; name: string }
  | { event: "progress"; pass: number; current_pos: number; bytes_total: number; bytes_rescued: number; bytes_bad: number; bytes_pending: number }
  | { event: "pass_complete"; pass: number }
  | { event: "done"; totals?: Totals; files_found?: number; bytes_written?: number; files?: number; deleted?: number; extracted?: number; failed?: number }
  | { event: "error"; message: string }
  | { event: "phase"; message: string }
  | { event: "status"; size: number; current_pos: number; totals: Totals; blocks: Block[] }
  | { event: "scan_progress"; bytes_scanned: number; bytes_total: number }
  | { event: "result"; table_kind: string; partitions: Partition[] }
  | { event: "version"; version: string; contract: number }
  | { event: "devices"; elevated: boolean; devices: Device[] }
  | { event: "types"; types: CarveType[] }
  | { event: "carve_progress"; bytes_scanned: number; bytes_total: number; files_found: number }
  | { event: "files_progress"; records_read: number; records_total: number }
  | { event: "extract_progress"; files_done: number; files_total: number; bytes_done: number; bytes_total: number }
  | { event: "extracted"; id: number; path: string | null; size: number; complete: boolean; note: string | null };

export interface Exit { job: string; code: number | null; stderr: string }

export type Job = "image" | "scan" | "carve" | "files" | "extract";

export interface CliInfo {
  path: string; searched: string[]; found: boolean; version: string | null; contract: number | null;
  problem: string | null; platform: string; pkexec: boolean;
}

export interface Restore { source: string; dest: string; map: string; screen: string }
