// Copies the release sf-cli into src-tauri/binaries/ under the name Tauri expects for a
// sidecar (externalBin): sf-cli-<host target triple>[.exe]. Run after `cargo build --release -p sf-cli`.
import { execFileSync } from "node:child_process";
import { copyFileSync, existsSync, mkdirSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const exe = process.platform === "win32" ? ".exe" : "";
const src = join(root, "..", "target", "release", `sf-cli${exe}`);

const info = execFileSync("rustc", ["-vV"], { encoding: "utf8" });
const triple = /^host:\s*(\S+)\s*$/m.exec(info)?.[1];
if (!triple) throw new Error("Could not read the host target triple from `rustc -vV`.");
if (!existsSync(src)) {
  throw new Error(`${src} not found. Build it first: cargo build --release -p sf-cli (in the repo root).`);
}

const dir = join(root, "src-tauri", "binaries");
mkdirSync(dir, { recursive: true });
const dest = join(dir, `sf-cli-${triple}${exe}`);
copyFileSync(src, dest);
console.log(`staged ${src}\n    -> ${dest}`);
