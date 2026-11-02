import { test } from "vitest";
import assert from "node:assert/strict";
import { FileStore, ORPHANS, crumbs, splitPath, joinPath, noFilter, kindOf, type Filter } from "./filestore";
import type { FileRow } from "./contract";

const row = (id: number, path: string | null, o: Partial<FileRow> = {}): FileRow =>
  ({ id, name: path ? path.split("/").pop()! : `orphan${id}`, path, is_dir: false, size: 10, deleted: false, recoverable: "full", ...o });

function sample() {
  const s = new FileStore();
  s.addMany([
    row(5, "/", { name: ".", is_dir: true }),
    row(10, "/Users", { is_dir: true }),
    row(11, "/Users/terra", { is_dir: true }),
    row(12, "/Users/terra/b.JPG", { deleted: true }),
    row(13, "/Users/terra/a.txt", { recoverable: "partial" }),
    row(14, "/Users/terra/Docs", { is_dir: true }),
    row(15, "/top.bin", { deleted: true, recoverable: "none" }),
    row(16, null, { name: "lost.doc", deleted: true, recoverable: "partial" }),
  ]);
  return s;
}
const f = (o: Partial<Filter> = {}): Filter => ({ deletedOnly: false, recoverableOnly: false, text: "", kinds: [], ...o });

test("splitPath handles root, nesting and sloppy slashes", () => {
  assert.deepEqual(splitPath("/a"), { dir: "/", leaf: "a" });
  assert.deepEqual(splitPath("/a/b/c.txt"), { dir: "/a/b", leaf: "c.txt" });
  assert.deepEqual(splitPath("a/b/"), { dir: "/a", leaf: "b" });
  assert.deepEqual(splitPath("/"), { dir: "/", leaf: "" });
  assert.equal(joinPath("/", "x"), "/x");
  assert.equal(joinPath("/a", "x"), "/a/x");
});

test("tree is built from paths; root is not listed; orphans are separate", () => {
  const s = sample();
  assert.equal(s.count, 7); // root entry skipped
  assert.deepEqual(s.list("/").map((i) => s.names[i]), ["Users", "top.bin"]);
  assert.deepEqual(s.list("/Users").map((i) => s.names[i]), ["terra"]);
  assert.equal(s.orphanCount, 1);
  assert.deepEqual(s.list(ORPHANS).map((i) => s.names[i]), ["lost.doc"]);
  assert.deepEqual(s.list("/nope"), []);
});

test("folders sort first, then names case-insensitively", () => {
  const s = sample();
  assert.deepEqual(s.list("/Users/terra").map((i) => s.names[i]), ["Docs", "a.txt", "b.JPG"]);
});

test("sorted list refreshes when a folder gains entries", () => {
  const s = sample();
  assert.equal(s.list("/Users/terra").length, 3);
  s.add(row(20, "/Users/terra/0first.txt"));
  const names = s.list("/Users/terra").map((i) => s.names[i]);
  assert.equal(names.length, 4);
  assert.equal(names[1], "0first.txt");
});

test("sortBy keeps folders first and orders by size, type, date or name, either way", () => {
  const s = new FileStore();
  s.addMany([
    row(1, "/b.zip", { size: 50, modified: "2026-01-02T00:00:00Z" }),
    row(2, "/a.txt", { size: 900, modified: "2026-03-01T00:00:00Z" }),
    row(3, "/dir", { is_dir: true, size: 0 }),
    row(4, "/c.JPG", { size: 5, modified: null }),
  ]);
  const names = (key: "name" | "type" | "modified" | "size", desc = false) => s.sortBy(s.list("/"), { key, desc }).map((i) => s.names[i]);
  assert.deepEqual(names("size", true), ["dir", "a.txt", "b.zip", "c.JPG"]);
  assert.deepEqual(names("size"), ["dir", "c.JPG", "b.zip", "a.txt"]);
  assert.deepEqual(names("type"), ["dir", "c.JPG", "a.txt", "b.zip"]);
  assert.deepEqual(names("modified", true), ["dir", "a.txt", "b.zip", "c.JPG"]);
  assert.deepEqual(names("name", true), ["dir", "c.JPG", "b.zip", "a.txt"]);
});

test("type filter: picks files by kind, never folders", () => {
  const s = sample();
  assert.equal(kindOf("b.JPG"), "image");
  assert.equal(kindOf("README"), "other");
  assert.deepEqual(s.search(f({ kinds: ["image"] })).map((i) => s.names[i]), ["b.JPG"]);
  assert.deepEqual(s.search(f({ kinds: ["document", "other"] })).map((i) => s.names[i]).sort(), ["a.txt", "lost.doc", "top.bin"]);
  assert.equal(noFilter(f({ kinds: ["video"] })), false);
});

test("pathOf and folderOf", () => {
  const s = sample();
  const i = s.ids.indexOf(12);
  assert.equal(s.pathOf(i), "/Users/terra/b.JPG");
  assert.equal(s.folderOf(i), "/Users/terra");
  assert.equal(s.folderOf(s.ids.indexOf(16)), ORPHANS);
  assert.equal(s.pathOf(s.ids.indexOf(15)), "/top.bin");
});

test("filters: deleted only, recoverable only, text, and combinations", () => {
  const s = sample();
  const ids = (o: Parameters<typeof f>[0]) => s.search(f(o)).map((i) => s.ids[i]);
  assert.deepEqual(ids({ deletedOnly: true }), [12, 15, 16]);
  assert.deepEqual(ids({ deletedOnly: true, recoverableOnly: true }), [12, 16]); // 15 has nothing left
  assert.deepEqual(ids({ text: "JPG" }), [12]); // case-insensitive
  assert.deepEqual(ids({ text: "  .TXT " }), [13]);
  assert.deepEqual(ids({ text: "zzz" }), []);
  assert.deepEqual(ids({ recoverableOnly: true }), [10, 11, 12, 13, 14, 16]);
  assert.equal(noFilter(f({ text: "  " })), true);
  assert.equal(noFilter(f({ deletedOnly: true })), false);
});

test("search sees entries added after the lowercase cache was built", () => {
  const s = sample();
  assert.equal(s.search(f({ text: "late" })).length, 0);
  s.add(row(30, "/late.txt"));
  assert.equal(s.search(f({ text: "late" })).length, 1);
});

test("counts deleted entries and resets on clear", () => {
  const s = sample();
  assert.equal(s.deletedCount, 3);
  s.clear();
  assert.equal(s.count, 0);
  assert.deepEqual(s.list("/"), []);
  assert.equal(s.deletedCount, 0);
});

test("hostile names stay inert data (escaping is the renderer's job; the store must not mangle them)", () => {
  const s = new FileStore();
  s.add(row(1, "/<img src=x onerror=alert(1)>", { name: "<img src=x onerror=alert(1)>" }));
  assert.equal(s.names[0], "<img src=x onerror=alert(1)>");
});

test("breadcrumbs", () => {
  assert.deepEqual(crumbs("/").map((c) => c.dir), ["/"]);
  assert.deepEqual(crumbs("/Users/terra").map((c) => [c.label, c.dir]), [["Volume root", "/"], ["Users", "/Users"], ["terra", "/Users/terra"]]);
  assert.equal(crumbs(ORPHANS).length, 1);
});

test("1.2 million entries load, filter and list in reasonable time", () => {
  const s = new FileStore();
  const t0 = performance.now();
  for (let i = 0; i < 1_200_000; i++) s.add({ id: i, name: `f${i}.dat`, path: `/d${i % 5000}/f${i}.dat`, is_dir: false, size: i, deleted: i % 100 === 0, recoverable: "full" });
  const hits = s.search(f({ deletedOnly: true, text: "f12" }));
  assert.equal(s.count, 1_200_000);
  assert.ok(hits.length > 0);
  assert.equal(s.list("/d7").length, 240);
  assert.ok(performance.now() - t0 < 15_000);
});
