// Turns sf-cli events (via the Rust side) into state changes.
import type { Ev, Exit, FileRow, Job } from "../lib/contract";
import { fmtBytes, fmtCount, percent } from "../lib/format";
import { pollStatus, stopPolling } from "./actions";
import { IDLE, useApp } from "./app";
import { tilesFromProgress, useImage } from "./image";
import { files, useRecover } from "./recover";
import { useScan } from "./scan";
import { useVerify } from "./verify";

const FILES_REDRAW_MS = 200;
let filesRedraw: ReturnType<typeof setTimeout> | undefined;

/** The file list grows by thousands of rows per second; redraw at most every FILES_REDRAW_MS. */
function scheduleFilesRedraw() {
  filesRedraw ??= setTimeout(flushFilesRedraw, FILES_REDRAW_MS);
}

function flushFilesRedraw() {
  clearTimeout(filesRedraw);
  filesRedraw = undefined;
  useRecover.setState((s) => ({ filesVersion: s.filesVersion + 1 }));
}

/** Events for a job this page didn't start (another browser tab on the dev bridge) must not land here. */
// The startup survey runs beside any user job and only adds facts about drives, so it is always welcome.
const ours = (job: string) => job === "survey" || useApp.getState().running === job;

export function onFiles(job: Job, rows: FileRow[]) {
  if (!ours(job)) return;
  if (job === "files") {
    files.addMany(rows);
    scheduleFilesRedraw();
  } else if (job === "carve") {
    const carved = rows.map((r) => ({ name: r.name, type: r.type ?? "", size: r.size ?? 0, offset: r.offset ?? 0, complete: r.complete !== false }));
    useRecover.setState((s) => ({ carve: { ...s.carve, rows: [...s.carve.rows, ...carved] } }));
  }
}

export function onEvent(job: Job, ev: Ev) {
  if (!ours(job)) return;
  if (ev.event === "error") {
    useApp.setState({ error: ev.message });
    if (job === "scan") useScan.setState((s) => ({ errors: { ...s.errors, [s.scanning.trim().toLowerCase()]: ev.message } }));
    return;
  }
  if (job === "survey") return onSurveyEvent(ev);
  if (ev.event === "start") useApp.setState({ jobMode: ev.mode });
  // A step with no numbers (opening a volume, preallocating a file): just say what it is.
  if (ev.event === "phase") return activity(`${ev.message}...`);

  switch (job) {
    case "image":
      return onImageEvent(ev);
    case "scan":
      return onScanEvent(ev);
    case "files":
      return onFilesEvent(ev);
    case "extract":
      return onExtractEvent(ev);
    case "carve":
      return onCarveEvent(ev);
    case "hash":
      return onHashEvent(ev);
  }
}

const activity = (text: string) => useApp.setState({ activity: text });

function onImageEvent(ev: Ev) {
  switch (ev.event) {
    case "start":
      activity(`${ev.resumed ? "Resuming" : "Imaging"} ${ev.source} (${fmtBytes(ev.size)}).`);
      break;
    case "pass_start":
      useImage.setState({ pass: `${ev.pass}: ${ev.name}` });
      activity(
        ev.name === "copy"
          ? "Pass 1: copying quickly, skipping past damaged areas to come back to later."
          : `Pass ${ev.pass}: going back to damaged areas and retrying sector by sector.`,
      );
      break;
    case "progress":
      useImage.setState({ tiles: tilesFromProgress(ev.bytes_rescued, ev.bytes_total, ev.bytes_bad, ev.bytes_pending) });
      break;
    case "done":
      activity("Finished. Every area has been tried.");
      break;
  }
}

function onScanEvent(ev: Ev) {
  switch (ev.event) {
    case "start":
      useScan.setState({ deviceSize: ev.size });
      break;
    case "scan_progress": {
      const pct = percent(ev.bytes_scanned, ev.bytes_total);
      useScan.setState({ pct });
      activity(`Scanning the whole drive for filesystems: ${pct.toFixed(1)}% (${fmtBytes(ev.bytes_scanned)} of ${fmtBytes(ev.bytes_total)}).`);
      break;
    }
    case "result":
      useScan.setState((s) => {
        const result = { kind: ev.table_kind, partitions: ev.partitions, source: s.scanning, deviceSize: s.deviceSize };
        const key = s.scanning.trim().toLowerCase();
        const { [key]: _cleared, ...errors } = s.errors;
        return { result, bySource: { ...s.bySource, [key]: result }, errors };
      });
      break;
  }
}

function onFilesEvent(ev: Ev) {
  switch (ev.event) {
    case "files_progress":
      useRecover.setState({ recordsRead: ev.records_read, recordsTotal: ev.records_total });
      activity(`Reading the file table: ${fmtCount(ev.records_read)} of ${fmtCount(ev.records_total)}.`);
      break;
    case "done":
      useRecover.setState({
        doneInfo: `Finished. ${fmtCount(ev.files ?? files.count)} files and folders, ${fmtCount(ev.deleted ?? files.deletedCount)} of them deleted.`,
      });
      break;
  }
}

function onExtractEvent(ev: Ev) {
  switch (ev.event) {
    case "files_progress":
      // extract reads the file table first; its progress bar only follows extract_progress.
      activity(`Reading the file table before extracting: ${fmtCount(ev.records_read)} of ${fmtCount(ev.records_total)}.`);
      break;
    case "extract_progress":
      useRecover.setState((s) => ({
        extract: { ...s.extract, filesDone: ev.files_done, filesTotal: ev.files_total, bytesDone: ev.bytes_done, bytesTotal: ev.bytes_total },
      }));
      activity(`Extracting: ${fmtCount(ev.files_done)} of ${fmtCount(ev.files_total)} files.`);
      break;
    case "extracted":
      useRecover.setState((s) => ({
        extract: { ...s.extract, rows: [...s.extract.rows, { path: ev.path, size: ev.size, complete: ev.complete, note: ev.note }] },
      }));
      break;
    case "done":
      useRecover.setState((s) => {
        const incomplete = s.extract.rows.filter((r) => r.path !== null && !r.complete).length;
        const extracted = ev.extracted ?? s.extract.rows.length;
        const parts = [`${fmtCount(extracted)} extracted`];
        if (ev.failed) parts.push(`${fmtCount(ev.failed)} could not be written`);
        if (incomplete) parts.push(`${fmtCount(incomplete)} incomplete (see each row's note)`);
        return {
          extract: {
            ...s.extract,
            filesDone: s.extract.filesTotal,
            summary: `Finished. ${parts.join(", ")}. Saved to ${s.out.trim()}.`,
          },
        };
      });
      break;
  }
}

function onCarveEvent(ev: Ev) {
  switch (ev.event) {
    case "carve_progress":
      useRecover.setState((s) => ({ carve: { ...s.carve, scanned: ev.bytes_scanned, total: ev.bytes_total, found: ev.files_found } }));
      activity(`Carving: ${fmtBytes(ev.bytes_scanned)} of ${fmtBytes(ev.bytes_total)} scanned, ${fmtCount(ev.files_found)} files found.`);
      break;
    case "done":
      useRecover.setState((s) => ({
        carve: {
          ...s.carve,
          summary: `Finished. ${fmtCount(ev.files_found ?? s.carve.rows.length)} files found, ${fmtBytes(ev.bytes_written ?? 0)} saved to ${s.carveOut.trim()}.`,
        },
      }));
      break;
  }
}

function onSurveyEvent(ev: Ev) {
  if (ev.event !== "survey_result" && ev.event !== "survey_error") return;
  const key = ev.source.trim().toLowerCase();
  useScan.setState((s) => {
    const surveying = s.surveying.filter((k) => k !== key);
    if (ev.event === "survey_error") return { surveying, errors: { ...s.errors, [key]: ev.message } };
    const { [key]: _cleared, ...errors } = s.errors;
    const result = { kind: ev.table_kind, partitions: ev.partitions, source: ev.source, deviceSize: ev.size };
    return { surveying, errors, bySource: { ...s.bySource, [key]: result } };
  });
}

function onHashEvent(ev: Ev) {
  switch (ev.event) {
    case "start":
      useVerify.setState({ size: ev.size, done: 0 });
      break;
    case "hash_progress":
      useVerify.setState({ done: ev.bytes_done, size: ev.bytes_total });
      activity(`Computing SHA-256: ${fmtBytes(ev.bytes_done)} of ${fmtBytes(ev.bytes_total)} read (${percent(ev.bytes_done, ev.bytes_total).toFixed(1)}%).`);
      break;
    case "done":
      if (ev.sha256) {
        const sha256 = ev.sha256;
        useVerify.setState((s) => ({ results: [{ source: s.hashing, size: s.size, sha256, at: new Date() }, ...s.results] }));
      }
      break;
  }
}

type Outcome = "done" | "stopped" | "error";
const outcomeOf = (code: number | null): Outcome => (code === 0 ? "done" : code === null ? "stopped" : "error");

export function onExit(x: Exit) {
  if (!ours(x.job)) return;
  // Not a user job: nothing is "running", so leave the status line alone.
  if (x.job === "survey") return useScan.setState({ surveying: [] });
  const outcome = outcomeOf(x.code);
  // A non-zero exit that never sent an `error` event (crash, usage error) still needs explaining.
  if (outcome === "error" && !useApp.getState().error) {
    useApp.setState({ error: `sf-cli stopped with exit code ${x.code}.${x.stderr.trim() ? ` ${x.stderr.trim()}` : ""}` });
  }
  const said = (done: string, stopped: string, failed: string) => (outcome === "done" ? done : outcome === "stopped" ? stopped : failed);
  let text = IDLE;

  switch (x.job) {
    case "image":
      stopPolling();
      void pollStatus();
      text = said("Idle. Last run finished.", "Paused. The map was saved; press Resume to continue.", "Imaging stopped by an error.");
      break;
    case "scan":
      useScan.setState({ pct: 0 });
      text = said("Idle. Scan finished.", "Scan cancelled.", "Scan stopped by an error.");
      break;
    case "files":
      flushFilesRedraw();
      useRecover.setState({ listing: outcome });
      text = said("Idle. File list ready.", "File listing stopped.", "File listing stopped by an error.");
      break;
    case "extract":
      useRecover.setState((s) => ({
        extract: {
          ...s.extract,
          state: outcome,
          summary:
            outcome === "stopped"
              ? `Stopped. ${fmtCount(s.extract.rows.length)} files were written; the last one may be incomplete.`
              : outcome === "error" && !s.extract.summary
                ? "Extraction stopped by an error."
                : s.extract.summary,
        },
      }));
      text = said("Idle. Extraction finished.", "Extraction stopped.", "Extraction stopped by an error.");
      break;
    case "carve":
      useRecover.setState((s) => ({
        carve: {
          ...s.carve,
          state: outcome,
          summary:
            outcome === "stopped"
              ? `Stopped. ${fmtCount(s.carve.rows.length)} files were saved before stopping; the last one may be incomplete.`
              : outcome === "error" && !s.carve.summary
                ? "Carving stopped by an error."
                : s.carve.summary,
        },
      }));
      text = said("Idle. Carving finished.", "Carving stopped.", "Carving stopped by an error.");
      break;
    case "hash":
      useVerify.setState({ hashing: "" });
      text = said("Idle. Hash finished.", "Hashing stopped.", "Hashing stopped by an error.");
      break;
  }
  useApp.setState({ running: null, activity: text });
}
