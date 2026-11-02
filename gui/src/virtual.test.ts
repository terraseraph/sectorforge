import { test } from "node:test";
import assert from "node:assert/strict";
import { windowAt, MAX_SPACER } from "./virtual.ts";

test("empty list and zero viewport render nothing", () => {
  assert.equal(windowAt(0, 500, 28, 0).count, 0);
  assert.equal(windowAt(0, 0, 28, 100).count, 0);
});

test("small list: renders all rows, no shift at top", () => {
  const w = windowAt(0, 280, 28, 5);
  assert.deepEqual([w.first, w.count, w.offset, w.spacer], [0, 5, 0, 140]);
});

test("scrolled by a fractional row: first row is shifted up by the remainder", () => {
  const w = windowAt(28 * 100 + 10, 280, 28, 1000, 2);
  assert.equal(w.first, 98); // 100 - overscan
  assert.equal(w.offset, 98 * 28 - (28 * 100 + 10));
  assert.ok(w.count >= Math.ceil(280 / 28) + 4);
});

test("window never runs past the end and the last row reaches the bottom", () => {
  const total = 1000, rowH = 28, vh = 300;
  const maxScroll = total * rowH - vh;
  const w = windowAt(maxScroll + 5000, vh, rowH, total);
  assert.equal(w.first + w.count, total);
  const bottom = w.offset + w.count * rowH;
  assert.ok(Math.abs(bottom - vh) < 1e-6, `bottom ${bottom}`);
});

test("rendered rows stay a small constant even with 5 million entries", () => {
  const total = 5_000_000, rowH = 28, vh = 700;
  const spacer = windowAt(0, vh, rowH, total).spacer;
  assert.equal(spacer, MAX_SPACER); // 140M px would break browsers; compressed instead
  for (const frac of [0, 0.25, 0.5, 0.999, 1]) {
    const w = windowAt((spacer - vh) * frac, vh, rowH, total);
    assert.ok(w.count < 60, `count ${w.count}`);
    assert.ok(w.first >= 0 && w.first + w.count <= total);
  }
});

test("compressed scrolling is monotonic and reaches both ends", () => {
  const total = 5_000_000, rowH = 28, vh = 700;
  const start = windowAt(0, vh, rowH, total);
  const end = windowAt(MAX_SPACER - vh, vh, rowH, total);
  assert.equal(start.first, 0);
  assert.equal(end.first + end.count, total);
  let prev = -1;
  for (let s = 0; s <= MAX_SPACER - vh; s += (MAX_SPACER - vh) / 200) {
    const w = windowAt(s, vh, rowH, total);
    assert.ok(w.first >= prev);
    prev = w.first;
  }
});
