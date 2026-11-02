import { test } from "node:test";
import assert from "node:assert/strict";
import { cellsFromBlocks } from "./map.ts";

test("worst status wins within a slice", () => {
  const cells = cellsFromBlocks(
    [
      { pos: 0, size: 150, status: "finished" },
      { pos: 150, size: 10, status: "bad_sector" },
      { pos: 160, size: 40, status: "non_tried" },
    ],
    200, 2,
  );
  assert.deepEqual(cells, ["finished", "bad_sector"]);
});

test("amber beats grey, finished loses to grey", () => {
  const cells = cellsFromBlocks(
    [
      { pos: 0, size: 50, status: "finished" }, { pos: 50, size: 50, status: "non_tried" },
      { pos: 100, size: 50, status: "non_tried" }, { pos: 150, size: 50, status: "non_trimmed" },
    ],
    200, 2,
  );
  assert.deepEqual(cells, ["non_tried", "non_trimmed"]);
});
