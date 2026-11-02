import { fireEvent, render, screen } from "@testing-library/react";
import { expect, test, vi } from "vitest";
import type { Partition } from "../lib/contract";
import { PartitionPicker } from "./PartitionPicker";

const part = (start_lba: number, filesystem: string, label: string | null = null): Partition => ({
  start_lba,
  size_sectors: 2048,
  filesystem,
  confidence: "confident",
  source: "gpt",
  label,
});
const parts = [part(34, "unknown", "Microsoft reserved partition"), part(4096, "ntfs", "Data")];

test("one card per partition; clicking picks it; unusable ones are disabled with the reason", () => {
  const onSelect = vi.fn();
  render(
    <PartitionPicker id="t" parts={parts} size={8 << 20} source="E:\\disk.img" selected={null} onSelect={onSelect} unusable={(p) => (p.filesystem === "ntfs" ? null : "Carve only")} />,
  );
  expect(screen.getByRole("radio", { name: /Unknown/ })).toBeDisabled();
  expect(screen.getByText("Carve only")).toBeInTheDocument();
  fireEvent.click(screen.getByRole("radio", { name: /NTFS/ }));
  expect(onSelect).toHaveBeenCalledWith(4096);
});

test("the whole-source choice is checked when nothing is selected", () => {
  render(<PartitionPicker id="t" parts={parts} size={8 << 20} source="E:\\disk.img" selected={null} onSelect={() => {}} whole="The whole source" />);
  expect(screen.getByRole("radio", { name: /The whole source/ })).toHaveAttribute("aria-checked", "true");
});

test("before a scan it says so instead of showing an empty list", () => {
  render(<PartitionPicker id="t" parts={null} size={null} source="E:\\disk.img" selected={null} onSelect={() => {}} />);
  expect(screen.getByText("Partitions show here once the source is read.")).toBeInTheDocument();
});
