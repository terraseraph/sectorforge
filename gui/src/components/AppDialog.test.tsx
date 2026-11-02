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
