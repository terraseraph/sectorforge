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
