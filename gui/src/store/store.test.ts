import { beforeEach, describe, expect, test, vi } from "vitest";
import { resetStores } from "../test/reset";

vi.mock("../lib/api", async (orig) => {
  const real = await orig<typeof import("../lib/api")>();
  return {
    ...real,
    api: {
      startImage: vi.fn(),
      stopImage: vi.fn(),
      startScan: vi.fn(),
      startFiles: vi.fn(),
      startExtract: vi.fn(),
      getStatus: vi.fn(),
    },
    dialogs: { confirmReadDrive: vi.fn() },
  };
});

const { api, dialogs } = await import("../lib/api");
const { startImage, startListing, startScan, startExtract } = await import("./actions");
const { onEvent, onExit, onFiles } = await import("./events");
const { useApp, IDLE } = await import("./app");
const { useImage } = await import("./image");
const { useRecover, files, selection } = await import("./recover");
const { useScan } = await import("./scan");

beforeEach(() => {
  resetStores();
  vi.mocked(api.startImage).mockReset().mockResolvedValue(undefined);
  vi.mocked(api.startScan).mockReset().mockResolvedValue(undefined);
  vi.mocked(api.startFiles).mockReset().mockResolvedValue(undefined);
  vi.mocked(api.startExtract).mockReset().mockResolvedValue(undefined);
  vi.mocked(api.getStatus).mockReset().mockRejectedValue("no map");
  vi.mocked(dialogs.confirmReadDrive).mockReset().mockResolvedValue(true);
});

const fill = () => useApp.setState({ source: "E:\\disk.img", dest: "F:\\out.img", map: "F:\\out.map" });

describe("starting jobs", () => {
  test("image needs all three paths", async () => {
    useApp.setState({ source: "E:\\disk.img" });
    await startImage();
    expect(api.startImage).not.toHaveBeenCalled();
    expect(useApp.getState().error).toMatch(/source, a destination and a map/);
  });

  test("image sends sizes in bytes and is marked running before invoke resolves", async () => {
    fill();
    let runningDuringInvoke: unknown;
    vi.mocked(api.startImage).mockImplementation(async () => {
      runningDuringInvoke = useApp.getState().running;
    });
    await startImage();
    expect(runningDuringInvoke).toBe("image");
    expect(api.startImage).toHaveBeenCalledWith(expect.objectContaining({ chunkSize: 1024 * 1024, skipSize: 1024 * 1024, retries: 3 }));
  });

  test("a rejected start rolls back and shows the reason", async () => {
    fill();
    vi.mocked(api.startImage).mockRejectedValue("sf-cli was not found");
    await startImage();
    expect(useApp.getState()).toMatchObject({ running: null, activity: IDLE, error: "sf-cli was not found" });
  });

  test("the system drive is refused until explicitly confirmed", async () => {
    useApp.setState({
      source: "\\\\.\\PhysicalDrive0",
      dest: "F:\\out.img",
      map: "F:\\out.map",
      devices: [{ path: "\\\\.\\PhysicalDrive0", model: "SSD", serial: null, size: 1, sector_size: 512, bus: "nvme", removable: false, system: true }],
    });
    await startImage();
    expect(api.startImage).not.toHaveBeenCalled();
    useApp.setState({ systemConfirmed: "\\\\.\\physicaldrive0" });
    await startImage();
    expect(api.startImage).toHaveBeenCalled();
  });

  test("only one job at a time", async () => {
    useApp.setState({ running: "carve" });
    await startScan("E:\\disk.img", false);
    expect(api.startScan).not.toHaveBeenCalled();
    expect(useApp.getState().error).toMatch(/carve job/);
  });

  test("reading a physical drive asks first, and stops if declined", async () => {
    useRecover.setState({ source: "\\\\.\\PhysicalDrive1", sourceEdited: true, lba: 2048 });
    vi.mocked(dialogs.confirmReadDrive).mockResolvedValue(false);
    await startListing();
    expect(api.startFiles).not.toHaveBeenCalled();
    expect(useApp.getState().running).toBeNull();
  });

  test("extract uses the source and partition that were listed, not the current fields", async () => {
    useRecover.setState({ listedFor: { source: "E:\\a.img", lba: 2048 }, source: "E:\\other.img", sourceEdited: true, lba: 999, out: "F:\\rec" });
    useApp.setState({ running: "files" });
    onFiles("files", [{ id: 42, name: "a.txt", path: "/a.txt", size: 3 }]);
    useApp.setState({ running: null });
    selection.add(0);
    await startExtract();
    expect(api.startExtract).toHaveBeenCalledWith(expect.objectContaining({ source: "E:\\a.img", partitionLba: 2048, ids: [42] }));
  });
});

describe("events", () => {
  test("a phase event becomes the activity line", () => {
    useApp.setState({ running: "files" });
    onEvent("files", { event: "phase", message: "Opening the NTFS volume" });
    expect(useApp.getState().activity).toBe("Opening the NTFS volume...");
  });

  test("image progress fills the tiles, and a stopped exit reads as paused", () => {
    useApp.setState({ running: "image" });
    onEvent("image", { event: "progress", pass: 1, current_pos: 0, bytes_total: 1000, bytes_rescued: 250, bytes_bad: 0, bytes_pending: 750 });
    expect(useImage.getState().tiles.rescued).toBe("25.00%");
    onExit({ job: "image", code: null, stderr: "" });
    expect(useApp.getState()).toMatchObject({ running: null, activity: expect.stringMatching(/^Paused/) });
  });

  test("a crash without an error event is still explained", () => {
    useApp.setState({ running: "scan" });
    onExit({ job: "scan", code: 101, stderr: "thread 'main' panicked" });
    expect(useApp.getState().error).toBe("sf-cli stopped with exit code 101. thread 'main' panicked");
  });

  test("an error event wins over the generic exit message", () => {
    useApp.setState({ running: "scan" });
    onEvent("scan", { event: "error", message: "cannot open source X" });
    onExit({ job: "scan", code: 1, stderr: "" });
    expect(useApp.getState().error).toBe("cannot open source X");
  });

  test("events for a job this page did not start are ignored", () => {
    onEvent("scan", { event: "result", table_kind: "gpt", partitions: [] });
    onExit({ job: "scan", code: 0, stderr: "" });
    expect(useScan.getState().result).toBeNull();
    expect(useApp.getState().activity).toBe(IDLE);
  });

  test("a scan read error sticks to that drive until a later scan of it succeeds", () => {
    useApp.setState({ running: "scan" });
    useScan.setState({ scanning: "D3" });
    onEvent("scan", { event: "error", message: "I/O device error" });
    expect(useScan.getState().errors).toEqual({ d3: "I/O device error" });
    onEvent("scan", { event: "result", table_kind: "gpt", partitions: [] });
    expect(useScan.getState().errors).toEqual({});
    expect(useScan.getState().bySource.d3).toMatchObject({ kind: "gpt" });
  });

  test("survey results fill each drive in, even with no job running", () => {
    useScan.setState({ surveying: ["a", "b"] });
    onEvent("survey", { event: "survey_result", source: "A", size: 100, table_kind: "gpt", partitions: [] });
    onEvent("survey", { event: "survey_error", source: "B", message: "I/O device error" });
    expect(useScan.getState()).toMatchObject({ surveying: [], errors: { b: "I/O device error" }, bySource: { a: { kind: "gpt", deviceSize: 100 } } });
    onExit({ job: "survey", code: 0, stderr: "" });
    expect(useApp.getState()).toMatchObject({ running: null, activity: IDLE, error: "" });
  });

  test("the scan result remembers which source it belongs to", () => {
    useApp.setState({ running: "scan" });
    useScan.setState({ scanning: "E:\\disk.img" });
    onEvent("scan", { event: "start", source: "E:\\disk.img", mode: "file", size: 4096, sector_size: 512, resumed: false });
    onEvent("scan", { event: "result", table_kind: "mbr", partitions: [] });
    expect(useScan.getState().result).toMatchObject({ source: "E:\\disk.img", deviceSize: 4096, kind: "mbr" });
    expect(useApp.getState().jobMode).toBe("file");
  });

  test("extract items that could not be written count as failed, not incomplete", () => {
    useApp.setState({ running: "extract" });
    useRecover.setState({ out: "F:\\rec" });
    onEvent("extract", { event: "extracted", id: 1, path: "a.txt", size: 3, complete: true, note: null });
    onEvent("extract", { event: "extracted", id: 2, path: "b.txt", size: 3, complete: false, note: "part unreadable" });
    onEvent("extract", { event: "extracted", id: 3, path: null, size: 0, complete: false, note: "unknown id" });
    onEvent("extract", { event: "done", extracted: 2, failed: 1 });
    expect(useRecover.getState().extract.summary).toBe("Finished. 2 extracted, 1 could not be written, 1 incomplete (see each row's note). Saved to F:\\rec.");
  });

  test("file batches land in the store and redraw once listing ends", () => {
    useApp.setState({ running: "files" });
    const before = useRecover.getState().filesVersion;
    onFiles("files", [
      { id: 5, name: "Users", path: "/Users", is_dir: true },
      { id: 6, name: "a.jpg", path: "/Users/a.jpg", deleted: true, size: 10 },
    ]);
    expect(files.count).toBe(2);
    onExit({ job: "files", code: 0, stderr: "" });
    expect(useRecover.getState().filesVersion).toBeGreaterThan(before);
    expect(useRecover.getState().listing).toBe("done");
  });
});
