import { test } from "vitest";
import assert from "node:assert/strict";
import { findDevice, systemDriveFor, deviceSummary, deviceTitle, busLabel, suggestImageName, mapPathFor } from "./devices";
import type { Device } from "./contract";

const d = (path: string, o: Partial<Device> = {}): Device =>
  ({ path, model: "WDC WD20EZRZ", serial: null, size: 2000398934016, sector_size: 512, bus: "sata", removable: false, system: false, ...o });
const devs = [d("\\\\.\\PhysicalDrive0", { system: true, model: "Samsung 980", bus: "nvme" }), d("\\\\.\\PhysicalDrive1"), d("/dev/sdc", { model: null, size: null, bus: "weird" })];

test("typed paths match devices case-insensitively, ignoring stray spaces", () => {
  assert.equal(findDevice(devs, "\\\\.\\PHYSICALDRIVE1 ")?.path, "\\\\.\\PhysicalDrive1");
  assert.equal(findDevice(devs, "C:\\disk.img"), undefined);
  assert.equal(findDevice(devs, ""), undefined);
});

test("system drive is detected however its path was typed", () => {
  assert.ok(systemDriveFor(devs, "\\\\.\\physicaldrive0"));
  assert.equal(systemDriveFor(devs, "\\\\.\\PhysicalDrive1"), undefined);
});

test("unknown model, size and bus get honest labels", () => {
  assert.equal(deviceTitle(devs[2]), "Unknown drive");
  assert.equal(deviceSummary(devs[2]), "Unknown drive, size unknown, weird");
  assert.equal(deviceSummary(devs[0]), "Samsung 980, 1.82 TiB, NVMe");
  assert.equal(busLabel(""), "Unknown");
});

test("suggested image names: drive model and date, or the image's own name", () => {
  const day = new Date(2026, 10, 2);
  assert.equal(suggestImageName(devs, "\\\\.\\PhysicalDrive0", day), "Samsung_980-2026-11-02.img");
  assert.equal(suggestImageName(devs, "/dev/sdc", day), "Unknown_drive-2026-11-02.img");
  assert.equal(suggestImageName(devs, "E:\\rescue\\old disk.vhd", day), "old_disk-copy.img");
  assert.equal(suggestImageName(devs, "", day), "disk-2026-11-02.img");
  assert.equal(mapPathFor("E:\\rescue\\disk.img"), "E:\\rescue\\disk.map");
  assert.equal(mapPathFor("E:\\v1.2\\disk"), "E:\\v1.2\\disk.map");
});
