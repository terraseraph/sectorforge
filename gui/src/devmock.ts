// Development only: stands in for the Rust side when the page runs in a plain browser
// (`npm run dev`, no Tauri window), so layout and the million-entry list can be checked.
// Never imported by production builds (see the DEV guard in main.ts).
// Query options: ?cli=missing|old  ?elevated=1  ?n=<number of files to list>
const q = new URLSearchParams(location.search);
const callbacks = new Map<number, (e: unknown) => void>();
const listeners = new Map<string, number[]>();
let nextId = 1;
const timers: Record<string, number> = {};

function emit(event: string, payload: unknown) {
  for (const cb of listeners.get(event) ?? []) callbacks.get(cb)?.({ event, id: 0, payload });
}
const exit = (job: string, code: number | null) => emit("sf-exit", { job, code, stderr: "" });
const ev = (job: string, event: unknown) => emit("sf-event", { job, event });

const devices = [
  { path: "\\\\.\\PhysicalDrive0", model: "Samsung SSD 980 PRO 1TB", serial: "S5GX", size: 1000204886016, sector_size: 512, bus: "nvme", removable: false, system: true },
  { path: "\\\\.\\PhysicalDrive1", model: "WDC WD20EZRZ-00Z5HB0", serial: "WD-1", size: 2000398934016, sector_size: 512, bus: "sata", removable: false, system: false },
  { path: "\\\\.\\PhysicalDrive2", model: null, serial: null, size: null, sector_size: null, bus: "usb", removable: true, system: false },
];
const partitions = [
  { start_lba: 2048, size_sectors: 1447256064, filesystem: "ntfs", confidence: "confident", source: "mbr", label: null },
  { start_lba: 1447258112, size_sectors: null, filesystem: "fat32", confidence: "rebuilt", source: "signature_scan", label: "<b>USB</b>" },
];

function stream(job: string, run: () => void) {
  run();
}

function listFiles() {
  const total = Number(q.get("n") ?? 1_200_000);
  let sent = 0;
  ev("files", { event: "start", source: "E:\\image.img", mode: "file", size: 2000398934016, sector_size: 512 });
  const tick = () => {
    const rows = [];
    for (let k = 0; k < 4000 && sent < total; k++, sent++) {
      const i = sent;
      const dir = `/Users/user${i % 7}/Folder${i % 400}`;
      if (i === 7) { rows.push({ id: 40, name: "Users", path: "/Users", is_dir: true, size: 0, deleted: false, modified: "2024-01-02T03:04:00Z", recoverable: "full" }); continue; }
      if (i < 7 && i !== 3) { rows.push({ id: 50 + i, name: `user${i}`, path: `/Users/user${i}`, is_dir: true, size: 0, deleted: false, modified: "2024-01-02T03:04:00Z", recoverable: "full" }); continue; }
      if (i === 3) { rows.push({ id: 900000, name: "<img src=x onerror=alert(1)>.jpg", path: "/Users/user0/<img src=x onerror=alert(1)>.jpg", is_dir: false, size: 1234, deleted: true, modified: "2024-03-11T09:12:00Z", recoverable: "full" }); continue; }
      if (i % 2000 === 1999) { rows.push({ id: 1000000 + i, name: `lost${i}.doc`, path: null, is_dir: false, size: 2048, deleted: true, recoverable: "partial" }); continue; }
      if (i % 40 === 0) { rows.push({ id: 100 + i, name: `Folder${i / 40}`, path: `/Users/user${i % 7}/Folder${i / 40}`, is_dir: true, size: 0, deleted: false, modified: "2024-01-02T03:04:00Z", recoverable: "full" }); continue; }
      const deleted = i % 50 === 0;
      rows.push({ id: 100 + i, name: `IMG_${i}.JPG`, path: `${dir}/IMG_${i}.JPG`, is_dir: false, size: 1000 + (i % 5000) * 777, deleted, modified: "2024-03-11T09:12:00Z", recoverable: deleted ? (["full", "partial", "none"] as const)[i % 3] : "full" });
    }
    emit("sf-files", { job: "files", files: rows });
    ev("files", { event: "files_progress", records_read: sent, records_total: total });
    if (sent < total) timers.files = window.setTimeout(tick, 4);
    else { ev("files", { event: "done", files: total, deleted: Math.floor(total / 50) }); exit("files", 0); }
  };
  tick();
}

function extract(args: { ids: number[] }) {
  const n = args.ids.length;
  let k = 0;
  const tick = () => {
    k++;
    ev("extract", { event: "extracted", id: k, path: `Users/user${k % 3}/<b>Folder</b>/IMG_${k}.JPG`, size: 123456 * k, complete: k % 4 !== 0, note: k % 4 === 0 ? "Part of this file was unreadable and has been left blank." : null });
    ev("extract", { event: "extract_progress", files_done: k, files_total: Math.min(n, 60), bytes_done: k * 1e6, bytes_total: 60e6 });
    if (k < Math.min(n, 60)) timers.extract = window.setTimeout(tick, 60);
    else { ev("extract", { event: "done", extracted: k, failed: 0 }); exit("extract", 0); }
  };
  tick();
}

function carve() {
  let k = 0;
  const tick = () => {
    k++;
    emit("sf-files", { job: "carve", files: [{ name: `jpg/f${k * 2048}.jpg`, type: k % 2 ? "jpg" : "png", size: 40000 * k, offset: 1048576 * k, complete: k % 5 !== 0 }] });
    ev("carve", { event: "carve_progress", bytes_scanned: k * 1e9, bytes_total: 40e9, files_found: k });
    if (k < 40) timers.carve = window.setTimeout(tick, 100);
    else { ev("carve", { event: "done", files_found: k, bytes_written: 1e8 }); exit("carve", 0); }
  };
  tick();
}

function handle(cmd: string, args: Record<string, any>): unknown {
  switch (cmd) {
    case "plugin:event|listen": { const id = nextId++; const l = listeners.get(args.event) ?? []; l.push(args.handler); listeners.set(args.event, l); return id; }
    case "plugin:event|unlisten": return null;
    case "plugin:dialog|ask": return true;
    case "plugin:dialog|open": return args.options?.directory ? "D:\\Recovered" : "E:\\image.img";
    case "plugin:dialog|save": return "E:\\rescue\\disk.img";
    case "startup_restore": return null;
    case "cli_info": {
      const mode = q.get("cli");
      const base = { path: "C:\\Apps\\SectorForge\\sf-cli.exe", searched: ["SF_CLI_PATH (not set)", "next to SectorForge: C:\\Apps\\SectorForge", "your PATH"], found: true, version: "0.2.0", contract: 2, problem: null as string | null, platform: "windows", pkexec: false };
      if (mode === "missing") return { ...base, found: false, version: null, contract: null, problem: "SectorForge could not find sf-cli, the program that does the actual reading and copying. Put sf-cli.exe in the same folder as SectorForge, or set SF_CLI_PATH to its full path." };
      if (mode === "old") return { ...base, version: "0.1.0", contract: 1, problem: "SectorForge found sf-cli at C:\\Old\\sf-cli.exe (version 0.1.0), which speaks contract 1, but this app needs contract 2. Put the sf-cli that came in the same download next to SectorForge, or point the SF_CLI_PATH environment variable at the right file." };
      return base;
    }
    case "list_devices": return { event: "devices", elevated: q.get("elevated") === "1", devices };
    case "list_types": return { event: "types", types: [["jpg", "JPEG image"], ["png", "PNG image"], ["pdf", "PDF document"], ["zip", "ZIP archive"], ["docx", "Word document"], ["mp4", "MP4 video"]].map(([id, description]) => ({ id, description })) };
    case "get_status": throw "No map file yet.";
    case "restart_as_admin": throw "Windows did not give SectorForge Administrator rights (the permission prompt was declined).";
    case "start_scan":
      setTimeout(() => { ev("scan", { event: "start", source: args.source, mode: "file", size: 2000398934016, sector_size: 512 }); ev("scan", { event: "result", table_kind: "mbr", partitions }); exit("scan", 0); }, 300);
      return null;
    case "start_files": stream("files", listFiles); return null;
    case "start_extract": extract(args.a); return null;
    case "start_carve": carve(); return null;
    case "stop_files": case "stop_extract": case "stop_carve": {
      const job = cmd.slice(5);
      clearTimeout(timers[job]); setTimeout(() => exit(job, null), 20);
      return null;
    }
    default: throw `devmock: ${cmd} is not mocked`;
  }
}

(window as any).__TAURI_INTERNALS__ = {
  transformCallback(cb: (e: unknown) => void) { const id = nextId++; callbacks.set(id, cb); return id; },
  unregisterCallback(id: number) { callbacks.delete(id); },
  invoke: async (cmd: string, args: Record<string, any> = {}) => handle(cmd, args),
  metadata: { currentWindow: { label: "main" }, currentWebview: { label: "main", windowLabel: "main" } },
};
(window as any).__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener() {} };
export {};
