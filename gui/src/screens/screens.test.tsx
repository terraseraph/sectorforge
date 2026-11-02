import { fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, test, vi } from "vitest";
import { resetStores } from "../test/reset";

vi.mock("../lib/api", async (orig) => {
  const real = await orig<typeof import("../lib/api")>();
  return { ...real, api: { listDevices: vi.fn(), restartAsAdmin: vi.fn(), getStatus: vi.fn() }, dialogs: {} };
});

const { useApp } = await import("../store/app");
const { useRecover, files, selection } = await import("../store/recover");
const { onFiles } = await import("../store/events");
const { ImageScreen } = await import("./ImageScreen");
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
});

describe("file browser", () => {
  const listing = () => {
    useRecover.setState({ listing: "done" });
    onFiles("files", [
      { id: 5, name: "Users", path: "/Users", is_dir: true },
      { id: 6, name: '<img src=x onerror="alert(1)">.jpg', path: '/Users/<img src=x onerror="alert(1)">.jpg', deleted: true, size: 1234, recoverable: "partial" },
      { id: 7, name: "notes.txt", path: "/Users/notes.txt", size: 10 },
      { id: 8, name: "lost.doc", path: null, deleted: true, size: 5 },
    ]);
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
});
