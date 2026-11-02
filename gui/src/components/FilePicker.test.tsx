import { fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, expect, test, vi } from "vitest";

vi.mock("../lib/api", async (orig) => {
  const real = await orig<typeof import("../lib/api")>();
  return { ...real, api: { listPlaces: vi.fn(), listDir: vi.fn(), createFolder: vi.fn() } };
});

const { api, dialogs } = await import("../lib/api");
const { FilePicker } = await import("./FilePicker");
const { usePicker } = await import("../store/picker");

const GB = 1024 ** 3;

beforeEach(() => {
  usePicker.setState({ req: null, lastDir: null });
  vi.mocked(api.listPlaces).mockResolvedValue([
    { path: "E:\\", label: "Data", free: 40 * GB, total: 465 * GB },
    { path: "G:\\", label: "Big", free: 800 * GB, total: 3700 * GB },
  ]);
  vi.mocked(api.listDir).mockImplementation(async (p) => (p === "G:\\" ? [{ name: "rescue", dir: true, size: 0 }, { name: "old.img", dir: false, size: 5 }] : []));
});

test("save: pick a drive and folder; the suggested name is kept; small drives are flagged", async () => {
  render(<FilePicker />);
  const path = dialogs.saveFile("Destination image file", "TOSHIBA-2026-11-02.img", 500 * GB);

  const e = await screen.findByRole("button", { name: /E:/ });
  expect(e).toHaveClass("short");
  expect(screen.getByRole("button", { name: /G:/ })).not.toHaveClass("short");

  fireEvent.click(screen.getByRole("button", { name: /G:/ }));
  fireEvent.click(await screen.findByRole("option", { name: /rescue/ }));
  expect(await screen.findByText("This folder is empty.")).toBeInTheDocument();
  expect(screen.getByLabelText("File name")).toHaveValue("TOSHIBA-2026-11-02.img");
  fireEvent.click(screen.getByRole("button", { name: "Save here" }));
  expect(await path).toBe("G:\\rescue\\TOSHIBA-2026-11-02.img");
  expect(usePicker.getState().lastDir).toBe("G:\\rescue");
});

test("open: picking a file returns its path; Cancel returns null", async () => {
  render(<FilePicker />);
  const path = dialogs.openFile("Choose an image file");
  fireEvent.click(await screen.findByRole("button", { name: /G:/ }));
  fireEvent.click(await screen.findByRole("option", { name: /old\.img/ }));
  fireEvent.click(screen.getByRole("button", { name: "Open" }));
  expect(await path).toBe("G:\\old.img");

  const cancelled = dialogs.openFolder("Choose a folder");
  fireEvent.click(await screen.findByRole("button", { name: "Cancel" }));
  expect(await cancelled).toBeNull();
});
