/** 1536 -> "1.50 KiB". `null` (unknown size) -> "unknown". */
export function fmtBytes(n: number | null | undefined): string {
  if (n === null || n === undefined) return "unknown";
  const units = ["B", "KiB", "MiB", "GiB", "TiB"];
  let i = 0;
  while (n >= 1024 && i < units.length - 1) {
    n /= 1024;
    i++;
  }
  return `${i ? n.toFixed(n < 10 ? 2 : 1) : n} ${units[i]}`;
}

export const fmtCount = (n: number) => n.toLocaleString("en-US");

/** "2024-03-11T09:12:00Z" -> "2024-03-11 09:12" (UTC; plain slicing because it runs per visible row). */
export function fmtDate(iso: string | null | undefined): string {
  return iso && iso.length >= 16 ? `${iso.slice(0, 10)} ${iso.slice(11, 16)}` : "";
}

/** 0..100, safe for a zero or unknown total. */
export const percent = (part: number, whole: number) => (whole > 0 ? Math.max(0, Math.min(100, (part / whole) * 100)) : 0);

/** `\\.\PhysicalDriveN` (Windows) or `/dev/...` (Linux) is a physical drive; anything else is an image file. */
export const isDevicePath = (s: string) => s.startsWith("\\\\.\\") || s.startsWith("/dev/");
