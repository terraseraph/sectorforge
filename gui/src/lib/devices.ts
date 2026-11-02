import type { Device } from "./contract";
import { fmtBytes } from "./format";

const norm = (p: string) => p.trim().toLowerCase();

/** The device a typed or picked source path refers to (Windows paths are case-insensitive). */
export function findDevice(devices: Device[], source: string): Device | undefined {
  const s = norm(source);
  return s ? devices.find((d) => norm(d.path) === s) : undefined;
}

/** The system drive, if `source` points at it. Imaging it needs an explicit confirmation. */
export function systemDriveFor(devices: Device[], source: string): Device | undefined {
  const d = findDevice(devices, source);
  return d?.system ? d : undefined;
}

const BUS: Record<string, string> = { sata: "SATA", nvme: "NVMe", usb: "USB", scsi: "SCSI", sd: "SD card", virtual: "Virtual", other: "Other" };
export const busLabel = (bus: string) => BUS[bus] ?? (bus || "Unknown");

export const deviceTitle = (d: Device) => d.model?.trim() || "Unknown drive";

/** One-line description: "WDC WD20EZRZ, 1.82 TiB, SATA". */
export function deviceSummary(d: Device): string {
  return [deviceTitle(d), d.size === null ? "size unknown" : fmtBytes(d.size), busLabel(d.bus)].join(", ");
}

/** File name for an image of `source`: "WDC_WD20EZRZ-2026-11-02.img" for a drive, "disk-copy.img" for an image file. */
export function suggestImageName(devices: Device[], source: string, today = new Date()): string {
  const d = findDevice(devices, source);
  const date = `${today.getFullYear()}-${String(today.getMonth() + 1).padStart(2, "0")}-${String(today.getDate()).padStart(2, "0")}`;
  const leaf = source.trim().split(/[\\/]/).pop()?.replace(/\.[^.]*$/, "") ?? "";
  const base = d ? `${deviceTitle(d)}-${date}` : leaf ? `${leaf}-copy` : `disk-${date}`;
  return base.replace(/[^A-Za-z0-9._-]+/g, "_").replace(/^_+|_+$/g, "") + ".img";
}

/** The map file kept next to an image: "E:\rescue\disk.img" -> "E:\rescue\disk.map". */
export const mapPathFor = (dest: string) => dest.trim().replace(/\.[^.\\/]*$/, "") + ".map";
