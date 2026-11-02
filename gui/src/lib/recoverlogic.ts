import type { Partition } from "./contract";

const SECTOR = 512; // *_lba / *_sectors fields are always 512-byte units

const samePath = (a: string, b: string) => a.trim().replace(/\//g, "\\").toLowerCase() === b.trim().replace(/\//g, "\\").toLowerCase();

/**
 * The imaging mapfile applies only when the recovery source is the image that map
 * describes (the Image screen's destination) and a map actually exists.
 */
export function mapForSource(source: string, imageDest: string, map: string, mapExists: boolean): string | undefined {
  return mapExists && map && imageDest && source && samePath(source, imageDest) ? map : undefined;
}

/** Byte range of a partition for `carve --start/--end`; `end` is omitted when the size is unknown. */
export function carveRange(p: Partition | undefined): { start?: number; end?: number } {
  if (!p) return {};
  const start = p.start_lba * SECTOR;
  return p.size_sectors === null ? { start } : { start, end: start + p.size_sectors * SECTOR };
}

export const isNtfs = (p: Partition) => p.filesystem === "ntfs";

export function partitionLabel(p: Partition, fmtSize: (n: number) => string): string {
  const size = p.size_sectors === null ? "size unknown" : fmtSize(p.size_sectors * SECTOR);
  return `${p.filesystem.toUpperCase()}${p.label ? ` "${p.label}"` : ""}, ${size}, starts at sector ${p.start_lba}`;
}
