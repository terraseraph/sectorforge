import { describe, expect, test } from "vitest";
import type { Partition } from "../lib/contract";
import { segments } from "./PartitionMap";

const part = (start_lba: number, size_sectors: number | null): Partition => ({
  start_lba, size_sectors, filesystem: "ntfs", confidence: "confident", source: "gpt", label: null,
});
const MiB = 2048; // sectors

describe("partition map segments", () => {
  test("partitions in disk order, with real gaps shown as unallocated and alignment slack ignored", () => {
    const size = 1000 * MiB * 512;
    const segs = segments([part(600 * MiB, 400 * MiB), part(1 * MiB, 300 * MiB)], size);
    expect(segs.map((s) => [s.kind, s.start / 512 / MiB, s.bytes / 512 / MiB])).toEqual([
      ["part", 1, 300],
      ["free", 301, 299],
      ["part", 600, 400],
    ]);
  });

  test("a partition of unknown size runs to the next one, or the end", () => {
    const size = 100 * MiB * 512;
    const segs = segments([part(0, null), part(50 * MiB, null)], size);
    expect(segs.map((s) => s.bytes / 512 / MiB)).toEqual([50, 50]);
  });
});
