import { listen } from "../lib/ipc";
import type { Ev, Exit, FileRow, Job } from "../lib/contract";
import { log } from "./console";
import { onEvent, onExit, onFiles } from "./events";

/** Quote arguments with spaces, so a console line can be pasted into a shell. */
const quote = (a: string) => (/[\s"]/.test(a) ? `"${a.replace(/"/g, '\\"')}"` : a);

/**
 * Subscribes to the Rust side's events. Called once at startup, outside React,
 * so StrictMode's double-run effects can't register listeners twice.
 * Everything also goes to the console, as close to sf-cli's own output as the bridge allows.
 */
export async function startBridge(): Promise<void> {
  await Promise.all([
    listen<{ job: Job; args: string[] }>("sf-cmd", (e) => log(e.payload.job, "cmd", `sf-cli ${e.payload.args.map(quote).join(" ")}`)),
    listen<{ job: Job; event: Ev }>("sf-event", (e) => {
      log(e.payload.job, "out", JSON.stringify(e.payload.event));
      onEvent(e.payload.job, e.payload.event);
    }),
    listen<{ job: Job; files: FileRow[] }>("sf-files", (e) => {
      log(e.payload.job, "files", `${e.payload.files.length} "file" events (batched; first: ${e.payload.files[0]?.path ?? e.payload.files[0]?.name ?? "?"})`);
      onFiles(e.payload.job, e.payload.files);
    }),
    listen<{ job: Job; line: string }>("sf-log", (e) => log(e.payload.job, "err", e.payload.line)),
    listen<Exit>("sf-exit", (e) => {
      const x = e.payload;
      log(x.job, "exit", x.code === null ? "stopped" : `exit code ${x.code}`);
      onExit(x);
    }),
  ]);
}
