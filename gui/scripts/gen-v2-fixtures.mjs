// Regenerates the hand-written v2_* NDJSON fixtures from the examples in docs/cli-contract.md.
// (JSON.stringify does the backslash escaping, which is easy to get wrong by hand.)
import { writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const dir = join(dirname(fileURLToPath(import.meta.url)), "..", "fixtures");
const out = (name, events) => writeFileSync(join(dir, name), events.map((e) => JSON.stringify(e)).join("\n") + "\n");
const IMG = "E:\\image.img";
const start = (extra = {}) => ({ event: "start", source: IMG, mode: "file", size: 2000398934016, sector_size: 512, ...extra });
const ntfs = { partition_lba: 2048, filesystem: "ntfs" };
const file = (id, parent_id, name, path, is_dir, size, deleted, modified, recoverable, extra = {}) =>
  ({ event: "file", id, parent_id, name, path, is_dir, size, deleted, modified, created: modified, recoverable, ...extra });

out("v2_version.ndjson", [{ event: "version", version: "0.2.0", contract: 2 }]);
out("v2_devices.ndjson", [{
  event: "devices", elevated: false,
  devices: [
    { path: "\\\\.\\PhysicalDrive0", model: "Samsung SSD 980 PRO 1TB", serial: "S5GXNX0T123456", size: 1000204886016, sector_size: 512, bus: "nvme", removable: false, system: true },
    { path: "\\\\.\\PhysicalDrive1", model: "WDC WD20EZRZ-00Z5HB0", serial: "WD-WCC4M0123456", size: 2000398934016, sector_size: 512, bus: "sata", removable: false, system: false },
    { path: "\\\\.\\PhysicalDrive2", model: null, serial: null, size: null, sector_size: null, bus: "usb", removable: true, system: false, future_field: 1 },
  ],
}]);
out("v2_types.ndjson", [{ event: "types", types: [{ id: "jpg", description: "JPEG image" }, { id: "png", description: "PNG image" }, { id: "pdf", description: "PDF document" }] }]);
out("v2_carve.ndjson", [
  start(),
  { event: "carve_progress", bytes_scanned: 4194304, bytes_total: 2000398934016, files_found: 3 },
  { event: "file", offset: 1048576, size: 2381702, type: "jpg", name: "jpg/f0002048.jpg", complete: true },
  { event: "file", offset: 9437184, size: 88000, type: "png", name: "png/f0018432.png", complete: false },
  { event: "done", files_found: 2, bytes_written: 2469702 },
]);
out("v2_files.ndjson", [
  start(ntfs),
  { event: "files_progress", records_read: 65536, records_total: 1200000 },
  file(5, 5, ".", "/", true, 0, false, null, "full"),
  file(4200, 5, "Users", "/Users", true, 0, false, "2024-03-01T00:00:00Z", "full"),
  file(4211, 4200, "IMG_4821.JPG", "/Users/IMG_4821.JPG", false, 4404019, true, "2024-03-11T09:12:00Z", "full"),
  file(4300, 9999, "lost.doc", null, false, 2048, true, null, "partial"),
  file(4301, 4200, "gone.bin", "/Users/gone.bin", false, 10, true, null, "none", { extra: "ignored" }),
  { event: "done", files: 5, deleted: 3 },
]);
out("v2_extract.ndjson", [
  start(ntfs),
  { event: "extract_progress", files_done: 1, files_total: 2, bytes_done: 4404019, bytes_total: 4500000 },
  { event: "extracted", id: 4211, path: "Users/terra/Pictures/IMG_4821.JPG", size: 4404019, complete: true, note: null },
  { event: "extracted", id: 4212, path: "_orphaned/lost.doc", size: 2048, complete: false, note: "Part of this file was unreadable and has been left blank." },
  { event: "done", extracted: 2, failed: 0 },
]);
out("v2_image_refusal.ndjson", [{ event: "error", message: "The destination is on the drive being imaged. Choose a folder on a different drive." }]);
