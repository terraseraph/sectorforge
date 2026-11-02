export const esc = (s: string) => s.replace(/[&<>"']/g, (c) => `&#${c.charCodeAt(0)};`);
export const $ = <T extends HTMLElement>(id: string) => document.getElementById(id) as T;

export function fmt(n: number | null | undefined): string {
  if (n === null || n === undefined) return "unknown";
  const u = ["B", "KiB", "MiB", "GiB", "TiB"];
  let i = 0;
  while (n >= 1024 && i < u.length - 1) { n /= 1024; i++; }
  return `${i ? n.toFixed(n < 10 ? 2 : 1) : n} ${u[i]}`;
}

export const isDevice = (s: string) => s.startsWith("\\\\.\\") || s.startsWith("/dev/");

/** "2024-03-11T09:12:00Z" -> "2024-03-11 09:12" (UTC; string slicing, because it runs per visible row). */
export function fmtDate(iso: string | null | undefined): string {
  return iso && iso.length >= 16 ? `${iso.slice(0, 10)} ${iso.slice(11, 16)}` : "";
}

export const fmtCount = (n: number) => n.toLocaleString("en-US");
