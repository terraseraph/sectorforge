// Paths on the computer running SectorForge (Windows "E:\a\b" or Unix "/a/b"), for the in-app file picker.

const sepOf = (p: string) => (p.includes("\\") || /^[A-Za-z]:/.test(p) ? "\\" : "/");

export function joinFs(dir: string, name: string): string {
  const sep = sepOf(dir);
  return dir.endsWith(sep) ? dir + name : dir + sep + name;
}

/** The folder above `path`, or null at a drive or filesystem root. */
export function parentOf(path: string): string | null {
  const sep = sepOf(path);
  const p = path.length > 1 && path.endsWith(sep) && !/^[A-Za-z]:\\$/.test(path) ? path.slice(0, -1) : path;
  if (p === "/" || /^[A-Za-z]:\\?$/.test(p)) return null;
  const i = p.lastIndexOf(sep);
  if (i < 0) return null;
  const up = p.slice(0, i);
  return up === "" ? "/" : /^[A-Za-z]:$/.test(up) ? up + "\\" : up;
}

/** "E:\rescue\disk.img" -> { dir: "E:\rescue", name: "disk.img" }; a bare name has no dir. */
export function splitFs(path: string): { dir: string | null; name: string } {
  const sep = sepOf(path);
  const i = path.lastIndexOf(sep);
  if (i < 0) return { dir: null, name: path };
  const dir = path.slice(0, i);
  return { dir: dir === "" ? "/" : /^[A-Za-z]:$/.test(dir) ? dir + "\\" : dir, name: path.slice(i + 1) };
}

/** Folder parts for breadcrumbs: "E:\a\b" -> ["E:\", "E:\a", "E:\a\b"]. */
export function crumbsOf(path: string): { label: string; path: string }[] {
  const out: { label: string; path: string }[] = [];
  for (let p: string | null = path; p; p = parentOf(p)) {
    const label = parentOf(p) === null ? p : (splitFs(p.replace(/[\\/]$/, "")).name || p);
    out.unshift({ label, path: p });
  }
  return out;
}
