import { test } from "vitest";
import assert from "node:assert/strict";
import { mapForSource, carveRange, isNtfs, partitionLabel } from "./recoverlogic";
import type { Partition } from "./contract";

const p = (o: Partial<Partition>): Partition => ({ start_lba: 2048, size_sectors: 1000, filesystem: "ntfs", confidence: "confident", source: "mbr", label: null, ...o });

test("the map is passed only when the source is the image that map describes", () => {
  assert.equal(mapForSource("E:\\rescue\\d.img", "E:\\rescue\\d.img", "E:\\rescue\\d.map", true), "E:\\rescue\\d.map");
  assert.equal(mapForSource("e:/rescue/D.IMG", "E:\\rescue\\d.img", "m", true), "m"); // slash and case differences
  assert.equal(mapForSource("E:\\other.img", "E:\\rescue\\d.img", "m", true), undefined);
  assert.equal(mapForSource("E:\\rescue\\d.img", "E:\\rescue\\d.img", "m", false), undefined); // no map written yet
  assert.equal(mapForSource("E:\\rescue\\d.img", "", "m", true), undefined);
  assert.equal(mapForSource("\\\\.\\PhysicalDrive1", "E:\\rescue\\d.img", "m", true), undefined);
});

test("carve range comes from a partition in 512-byte units, open-ended when size unknown", () => {
  assert.deepEqual(carveRange(undefined), {});
  assert.deepEqual(carveRange(p({})), { start: 2048 * 512, end: 3048 * 512 });
  assert.deepEqual(carveRange(p({ size_sectors: null })), { start: 2048 * 512 });
});

test("ntfs check and partition label", () => {
  assert.ok(isNtfs(p({})));
  assert.ok(!isNtfs(p({ filesystem: "fat32" })));
  assert.equal(partitionLabel(p({ label: "Data" }), (n) => `${n}B`), 'NTFS "Data", 512000B, starts at sector 2048');
  assert.match(partitionLabel(p({ size_sectors: null }), String), /size unknown/);
});
