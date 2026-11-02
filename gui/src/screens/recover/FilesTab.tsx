import { useEffect } from "react";
import { Button, Check, Field, Hint, Panel, ProgressBar, TextInput } from "../../components/ui";
import { dialogs } from "../../lib/api";
import { fmtBytes, fmtCount, percent } from "../../lib/format";
import { isNtfs, partitionLabel } from "../../lib/recoverlogic";
import { startExtract, startListing, startScan, stopExtract, stopListing } from "../../store/actions";
import { useApp } from "../../store/app";
import { files, selection, useRecover } from "../../store/recover";
import { partitionsFor, useScan } from "../../store/scan";
import { ExtractPanel } from "./ExtractPanel";
import { FileBrowser } from "./FileBrowser";

export function FilesTab({ source }: { source: string }) {
  const running = useApp((s) => s.running);
  const result = useScan((s) => s.result);
  const lba = useRecover((s) => s.lba);
  const deletedOnlyRun = useRecover((s) => s.deletedOnlyRun);
  const listing = useRecover((s) => s.listing);
  const parts = partitionsFor(result, source);

  // Keep a valid NTFS partition selected as scan results come and go.
  useEffect(() => {
    if (parts && (lba === null || !parts.some((p) => p.start_lba === lba))) {
      useRecover.setState({ lba: parts.find(isNtfs)?.start_lba ?? null });
    }
  }, [parts, lba]);

  return (
    <>
      <Panel label="Partition to list">
        <Field
          id="part"
          label="Partition"
          action={
            <Button disabled={!!running} onClick={() => void startScan(source, false)}>
              Scan source
            </Button>
          }
        >
          <select
            id="part"
            value={lba ?? ""}
            onChange={(e) => useRecover.setState({ lba: e.target.value === "" ? null : Number(e.target.value) })}
          >
            {parts && parts.length ? (
              parts.map((p) => (
                <option key={p.start_lba} value={p.start_lba} disabled={!isNtfs(p)}>
                  {partitionLabel(p, fmtBytes)}
                  {isNtfs(p) ? "" : " (file browsing supports NTFS only)"}
                </option>
              ))
            ) : (
              <option value="">No partitions known for this source yet. Press Scan source.</option>
            )}
          </select>
        </Field>
        <Check checked={deletedOnlyRun} onChange={(on) => useRecover.setState({ deletedOnlyRun: on })}>
          Only list deleted files (quicker on very large volumes)
        </Check>
        <div className="actions">
          <Button variant="primary" disabled={!!running} onClick={() => void startListing()}>
            List files
          </Button>
          <Button disabled={running !== "files"} onClick={stopListing}>
            Stop
          </Button>
        </div>
        <ListingProgress />
      </Panel>

      {listing !== "" && (
        <Panel label="Files found">
          <FileBrowser />
          <ExtractControls />
        </Panel>
      )}
      <ExtractPanel />
    </>
  );
}

function ListingProgress() {
  const listing = useRecover((s) => s.listing);
  const read = useRecover((s) => s.recordsRead);
  const total = useRecover((s) => s.recordsTotal);
  const doneInfo = useRecover((s) => s.doneInfo);
  useRecover((s) => s.filesVersion);
  const n = fmtCount(files.count);

  const text =
    listing === "running"
      ? `Reading the file table: ${fmtCount(read)} of ${total ? fmtCount(total) : "?"} records. ${n} files and folders so far.`
      : listing === "done"
        ? doneInfo
        : listing === "stopped"
          ? `Stopped early. ${n} entries were read, so the list is incomplete.`
          : listing === "error"
            ? `Listing failed. ${n} entries were read before it stopped.`
            : "";
  if (!text) return null;
  return (
    <div className="spaced">
      {listing === "running" && <ProgressBar value={percent(read, total)} label="File table progress" />}
      <Hint flush>{text}</Hint>
    </div>
  );
}

function ExtractControls() {
  const running = useApp((s) => s.running);
  const listing = useRecover((s) => s.listing);
  const out = useRecover((s) => s.out);
  useRecover((s) => s.selVersion);
  const disabled = !!running || listing === "running" || selection.size === 0 || !out.trim();

  return (
    <>
      <Field
        id="out"
        label="Extract to"
        action={
          <Button
            onClick={async () => {
              const d = await dialogs.openFolder("Choose where to save recovered files");
              if (d !== null) useRecover.setState({ out: d });
            }}
          >
            Choose
          </Button>
        }
      >
        <TextInput
          id="out"
          value={out}
          placeholder="A folder on a different drive from the one you are rescuing"
          onChange={(e) => useRecover.setState({ out: e.target.value })}
        />
      </Field>
      <div className="actions">
        <Button variant="primary" disabled={disabled} onClick={() => void startExtract()}>
          Extract selected
        </Button>
        <Button disabled={running !== "extract"} onClick={stopExtract}>
          Stop
        </Button>
        <span className="dim">Selecting a folder extracts everything inside it.</span>
      </div>
    </>
  );
}
