import { fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, test, vi } from "vitest";
import { resetStores } from "../test/reset";

vi.mock("../lib/api", async (orig) => {
  const real = await orig<typeof import("../lib/api")>();
  return { ...real, api: { listDevices: vi.fn(), restartAsAdmin: vi.fn(), getStatus: vi.fn(), startScan: vi.fn() }, dialogs: {} };
});

const { useImage } = await import("../store/image");
const { useApp } = await import("../store/app");
const { useRecover, files, selection } = await import("../store/recover");
const { onFiles } = await import("../store/events");
const { ImageScreen } = await import("./ImageScreen");
const { RecoverScreen } = await import("./recover/RecoverScreen");
const { api } = await import("../lib/api");
const { useScan } = await import("../store/scan");
const { FileBrowser } = await import("./recover/FileBrowser");

beforeEach(resetStores);

const systemDrive = {
  path: "\\\\.\\PhysicalDrive0",
  model: "Samsung SSD 980 PRO 1TB",
  serial: null,
  size: 1000204886016,
  sector_size: 512,
  bus: "nvme",
  removable: false,
  system: true,
};

describe("Image screen", () => {
  test("imaging the system drive needs an explicit tick", () => {
    useApp.setState({ devices: [systemDrive], devicesState: "loaded", elevated: true });
    render(<ImageScreen />);

    fireEvent.click(screen.getByRole("radio", { name: /Samsung SSD 980 PRO/ }));
    expect(screen.getByRole("radio", { name: /Samsung/ })).toHaveAttribute("aria-checked", "true");
    expect(screen.getByLabelText("Source")).toHaveValue("\\\\.\\PhysicalDrive0");
    expect(screen.getByRole("button", { name: "Start" })).toBeDisabled();

    fireEvent.click(screen.getByRole("checkbox", { name: /image the system drive anyway/ }));
    expect(screen.getByRole("button", { name: "Start" })).toBeEnabled();
  });

  test("without Administrator rights a drive offers a restart", () => {
    useApp.setState({ devices: [{ ...systemDrive, system: false }], devicesState: "loaded", elevated: false, source: systemDrive.path });
    useApp.setState({ cli: { path: "", searched: [], found: true, version: "0.2.0", contract: 2, problem: null, platform: "windows", pkexec: false } });
    render(<ImageScreen />);
    expect(screen.getByRole("button", { name: "Restart as Administrator" })).toBeInTheDocument();
  });

  test("a preset fills the advanced settings, shown in fitting units; editing makes them custom", () => {
    render(<ImageScreen />);
    expect(screen.getByRole("radio", { name: "Some bad sectors" })).toHaveAttribute("aria-checked", "true");
    fireEvent.click(screen.getByRole("radio", { name: "Badly damaged" }));
    expect(screen.getByLabelText("Read size (fast pass)")).toHaveValue(64);
    expect(screen.getAllByRole("combobox", { name: "Unit" })[0]).toHaveValue("1024");
    expect(screen.getByLabelText("Jump past a bad read")).toHaveValue(16);
    fireEvent.change(screen.getByLabelText("Retries per sector"), { target: { value: "5" } });
    expect(screen.getByText("Custom settings.")).toBeInTheDocument();
    expect(useImage.getState()).toMatchObject({ chunkBytes: 64 * 1024, skipBytes: 16 * 1024 * 1024, retries: 5 });
  });
});

describe("file browser", () => {
  const listing = () => {
    useRecover.setState({ listing: "done" });
    useApp.setState({ running: "files" });
    onFiles("files", [
      { id: 5, name: "Users", path: "/Users", is_dir: true },
      { id: 6, name: '<img src=x onerror="alert(1)">.jpg', path: '/Users/<img src=x onerror="alert(1)">.jpg', deleted: true, size: 1234, recoverable: "partial" },
      { id: 7, name: "notes.txt", path: "/Users/notes.txt", size: 10 },
      { id: 8, name: "lost.doc", path: null, deleted: true, size: 5 },
    ]);
    useApp.setState({ running: null });
    useRecover.setState((s) => ({ filesVersion: s.filesVersion + 1 }));
  };

  test("names off a damaged disk render as plain text", () => {
    listing();
    useRecover.setState({ folder: "/Users" });
    const { container } = render(<FileBrowser />);
    expect(screen.getByText('<img src=x onerror="alert(1)">.jpg')).toBeInTheDocument();
    expect(container.querySelector("img")).toBeNull();
  });

  test("folders open, and orphans get their own group", () => {
    listing();
    render(<FileBrowser />);
    fireEvent.click(screen.getByRole("button", { name: "Users" }));
    expect(useRecover.getState().folder).toBe("/Users");
    expect(screen.getByText("notes.txt")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Orphaned files (1)" }));
    expect(screen.getByText("lost.doc")).toBeInTheDocument();
  });

  test("select-all covers exactly what is shown", () => {
    listing();
    useRecover.setState({ folder: "/Users" });
    render(<FileBrowser />);
    fireEvent.click(screen.getByRole("checkbox", { name: "Select everything shown" }));
    expect([...selection].map((i) => files.names[i]).sort()).toEqual(['<img src=x onerror="alert(1)">.jpg', "notes.txt"]);
    expect(screen.getByText(/2 selected/)).toBeInTheDocument();
  });

  test("a deleted-only filter switches to a flat view of the whole partition", () => {
    listing();
    render(<FileBrowser />);
    fireEvent.click(screen.getByRole("checkbox", { name: "Deleted only" }));
    expect(screen.getByText("Showing matches from the whole partition.")).toBeInTheDocument();
    expect(screen.getByText("lost.doc")).toBeInTheDocument();
    expect(screen.queryByText("notes.txt")).toBeNull();
  });

  test("clicking a column header sorts the list; clicking again reverses it", () => {
    listing();
    render(<FileBrowser />);
    fireEvent.click(screen.getByRole("button", { name: "Name" }));
    expect(useRecover.getState().sort).toEqual({ key: "name", desc: false });
    fireEvent.click(screen.getByRole("button", { name: /^Name/ }));
    expect(useRecover.getState().sort).toEqual({ key: "name", desc: true });
    fireEvent.click(screen.getByRole("button", { name: "Size" }));
    expect(useRecover.getState().sort).toEqual({ key: "size", desc: true });
  });

  test("a type checkbox narrows the list to that kind of file", () => {
    listing();
    render(<FileBrowser />);
    fireEvent.click(screen.getByRole("checkbox", { name: "Images" }));
    expect(screen.getByText('<img src=x onerror="alert(1)">.jpg')).toBeInTheDocument();
    expect(screen.queryByText("notes.txt")).toBeNull();
  });
});

describe("Recover screen", () => {
  test("picking a drive reads its partition table straight away, once", () => {
    vi.mocked(api.startScan).mockReset().mockResolvedValue(undefined);
    useApp.setState({ devices: [{ ...systemDrive, system: false }], devicesState: "loaded", elevated: true });
    render(<RecoverScreen />);
    fireEvent.click(screen.getByRole("radio", { name: /Samsung SSD 980 PRO/ }));
    expect(api.startScan).toHaveBeenCalledWith(systemDrive.path, false);

    // Already read: picking it again doesn't read the drive again.
    useApp.setState({ running: null });
    useScan.setState({ scanning: "", result: { kind: "gpt", partitions: [], source: systemDrive.path, deviceSize: 1 } });
    fireEvent.click(screen.getByRole("radio", { name: /Samsung SSD 980 PRO/ }));
    expect(api.startScan).toHaveBeenCalledTimes(1);
  });
});
