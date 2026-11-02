import { test } from "vitest";
import assert from "node:assert/strict";
import { crumbsOf, joinFs, parentOf, splitFs } from "./paths";

test("Windows paths: join, parent, split, crumbs", () => {
  assert.equal(joinFs("E:\\", "rescue"), "E:\\rescue");
  assert.equal(joinFs("E:\\rescue", "a.img"), "E:\\rescue\\a.img");
  assert.equal(parentOf("E:\\rescue\\old"), "E:\\rescue");
  assert.equal(parentOf("E:\\rescue"), "E:\\");
  assert.equal(parentOf("E:\\rescue\\"), "E:\\");
  assert.equal(parentOf("E:\\"), null);
  assert.deepEqual(splitFs("E:\\rescue\\a.img"), { dir: "E:\\rescue", name: "a.img" });
  assert.deepEqual(splitFs("E:\\a.img"), { dir: "E:\\", name: "a.img" });
  assert.deepEqual(splitFs("a.img"), { dir: null, name: "a.img" });
  assert.deepEqual(crumbsOf("E:\\rescue\\old").map((c) => c.label), ["E:\\", "rescue", "old"]);
});

test("Unix paths: join, parent, split, crumbs", () => {
  assert.equal(joinFs("/", "home"), "/home");
  assert.equal(joinFs("/home/terra", "a.img"), "/home/terra/a.img");
  assert.equal(parentOf("/home/terra"), "/home");
  assert.equal(parentOf("/home"), "/");
  assert.equal(parentOf("/"), null);
  assert.deepEqual(splitFs("/mnt/a.img"), { dir: "/mnt", name: "a.img" });
  assert.deepEqual(splitFs("/a.img"), { dir: "/", name: "a.img" });
  assert.deepEqual(crumbsOf("/mnt/usb").map((c) => c.path), ["/", "/mnt", "/mnt/usb"]);
});
