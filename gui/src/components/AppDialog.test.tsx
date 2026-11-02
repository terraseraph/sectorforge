import { act, fireEvent, render, screen } from "@testing-library/react";
import { expect, test } from "vitest";
import { dialogs } from "../lib/api";
import { AppDialog } from "./AppDialog";

test("the read-drive question is an in-app dialog: OK says yes, Escape says no", async () => {
  render(<AppDialog />);

  let answer = act(() => dialogs.confirmReadDrive("list files"));
  expect(await screen.findByRole("dialog", { name: "Read the drive directly?" })).toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: "Read the drive" }));
  expect(await answer).toBe(true);
  expect(screen.queryByRole("dialog")).not.toBeInTheDocument();

  answer = act(() => dialogs.confirmReadDrive("list files"));
  fireEvent.keyDown(await screen.findByRole("dialog"), { key: "Escape" });
  expect(await answer).toBe(false);
});

test("in a browser tab, Browse asks for the path in the dialog", async () => {
  render(<AppDialog />);

  const path = act(() => dialogs.saveFile("Map file", "E:\\rescue\\disk.map"));
  const input = await screen.findByRole("textbox", { name: "Map file" });
  expect(input).toHaveValue("E:\\rescue\\disk.map");
  fireEvent.change(input, { target: { value: "F:\\disk.map" } });
  fireEvent.click(screen.getByRole("button", { name: "OK" }));
  expect(await path).toBe("F:\\disk.map");

  const cancelled = act(() => dialogs.openFile("Choose an image file"));
  fireEvent.click(await screen.findByRole("button", { name: "Cancel" }));
  expect(await cancelled).toBeNull();
});
