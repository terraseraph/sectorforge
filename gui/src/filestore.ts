import type { FileRow, Recoverable } from "./contract.ts";

/** Pseudo-folder holding files whose parent chain is broken (`path: null`). */
export const ORPHANS = ":orphans:"; // real folder keys always start with "/", so this cannot collide

export const F_DIR = 1, F_DELETED = 2;
export const REC: Record<Recoverable, number> = { none: 0, partial: 1, full: 2 };
export const REC_NAME = ["none", "partial", "full"] as const;

export interface Filter { deletedOnly: boolean; recoverableOnly: boolean; text: string }
export const noFilter = (f: Filter) => !f.deletedOnly && !f.recoverableOnly && f.text.trim() === "";

/** "/Users/a/b.txt" -> dir "/Users/a", leaf "b.txt". Tolerates missing leading or trailing slashes. */
export function splitPath(path: string): { dir: string; leaf: string } {
  let p = path.startsWith("/") ? path : "/" + path;
  while (p.length > 1 && p.endsWith("/")) p = p.slice(0, -1);
  const i = p.lastIndexOf("/");
  return { dir: i <= 0 ? "/" : p.slice(0, i), leaf: p.slice(i + 1) };
}

export const joinPath = (dir: string, name: string) => (dir === "/" ? "/" + name : dir + "/" + name);

export interface Crumb { label: string; dir: string }

/** Breadcrumbs for a folder: root, then each segment. */
export function crumbs(dir: string): Crumb[] {
  const out: Crumb[] = [{ label: "Volume root", dir: "/" }];
  if (dir === "/" || dir === ORPHANS) return out;
  let acc = "";
  for (const seg of dir.split("/").filter(Boolean)) {
    acc += "/" + seg;
    out.push({ label: seg, dir: acc });
  }
  return out;
}

/**
 * Column-oriented store for very large listings (a million+ entries): no per-file
 * object and no per-file path string; each file keeps its name plus an index into
 * a shared list of folder paths. Entries are addressed by insertion index.
 */
export class FileStore {
  ids: number[] = [];
  names: string[] = [];
  /** Index into `dirs`, or -1 for orphans. */
  dirOf: number[] = [];
  flags: number[] = [];
  recov: number[] = [];
  sizes: number[] = [];
  modified: (string | null)[] = [];
  deletedCount = 0;

  private dirs: string[] = [];
  private dirIdx = new Map<string, number>();
  private kids: number[][] = [];
  private orphans: number[] = [];
  private sorted = new Map<number, { n: number; list: number[] }>();
  private lower: string[] = [];

  get count() { return this.ids.length; }
  get orphanCount() { return this.orphans.length; }

  clear() {
    this.ids = []; this.names = []; this.dirOf = []; this.flags = []; this.recov = []; this.sizes = []; this.modified = [];
    this.dirs = []; this.kids = []; this.orphans = []; this.lower = [];
    this.dirIdx = new Map(); this.sorted = new Map(); this.deletedCount = 0;
  }

  private dirIndex(dir: string): number {
    let i = this.dirIdx.get(dir);
    if (i === undefined) { i = this.dirs.length; this.dirs.push(dir); this.kids.push([]); this.dirIdx.set(dir, i); }
    return i;
  }

  /** Adds one entry; returns its index, or -1 if it was the volume root (not listed). */
  add(r: FileRow): number {
    let dir = -1, name = r.name;
    if (r.path !== null && r.path !== undefined) {
      const { dir: d, leaf } = splitPath(r.path);
      if (leaf === "") return -1;
      dir = this.dirIndex(d);
      name = r.name || leaf;
    }
    const i = this.ids.length;
    this.ids.push(r.id ?? -1);
    this.names.push(name);
    this.dirOf.push(dir);
    const del = r.deleted ? F_DELETED : 0;
    this.flags.push((r.is_dir ? F_DIR : 0) | del);
    this.recov.push(REC[r.recoverable ?? "full"] ?? 2);
    this.sizes.push(r.size ?? 0);
    this.modified.push(r.modified ?? null);
    if (del) this.deletedCount++;
    (dir < 0 ? this.orphans : this.kids[dir]).push(i);
    return i;
  }

  addMany(rows: FileRow[]) { for (const r of rows) this.add(r); }

  isDir(i: number) { return (this.flags[i] & F_DIR) !== 0; }
  isDeleted(i: number) { return (this.flags[i] & F_DELETED) !== 0; }
  folderOf(i: number) { return this.dirOf[i] < 0 ? ORPHANS : this.dirs[this.dirOf[i]]; }
  /** Full path of an entry; orphans have only their name. */
  pathOf(i: number) { return this.dirOf[i] < 0 ? this.names[i] : joinPath(this.dirs[this.dirOf[i]], this.names[i]); }

  /** Folders first, then names, case-insensitively. Cached until the folder gains entries. */
  list(dir: string): number[] {
    if (dir === ORPHANS) return this.sortList(-1, this.orphans);
    const di = this.dirIdx.get(dir);
    return di === undefined ? [] : this.sortList(di, this.kids[di]);
  }

  private sortList(key: number, src: number[]): number[] {
    const hit = this.sorted.get(key);
    if (hit && hit.n === src.length) return hit.list;
    const keyed = src.map((i) => ({ i, d: this.flags[i] & F_DIR ? 0 : 1, k: this.names[i].toLowerCase() }));
    keyed.sort((a, b) => a.d - b.d || (a.k < b.k ? -1 : a.k > b.k ? 1 : 0));
    const list = keyed.map((x) => x.i);
    this.sorted.set(key, { n: src.length, list });
    return list;
  }

  /** Matches across the whole volume, in listing order. */
  search(f: Filter): number[] {
    const text = f.text.trim().toLowerCase();
    if (text) { for (let i = this.lower.length; i < this.names.length; i++) this.lower.push(this.names[i].toLowerCase()); }
    const out: number[] = [];
    for (let i = 0; i < this.ids.length; i++) {
      if (f.deletedOnly && !(this.flags[i] & F_DELETED)) continue;
      if (f.recoverableOnly && this.recov[i] === 0) continue;
      if (text && !this.lower[i].includes(text)) continue;
      out.push(i);
    }
    return out;
  }
}
