import { DrivePicker } from "../components/DrivePicker";
import { ErrorAlert, JobActivity } from "../components/Shell";
import { Badge, Button, Check, Field, Panel, ProgressBar, TextInput, type Tone } from "../components/ui";
import { dialogs } from "../lib/api";
import type { Partition } from "../lib/contract";
import { fmtBytes } from "../lib/format";
import { canBrowse } from "../lib/recoverlogic";
import { cancelScan, goTo, startScan } from "../store/actions";
import { useRecover } from "../store/recover";
import { useApp } from "../store/app";
import { type ScanResult, useScan } from "../store/scan";

/** `*_lba` / `*_sectors` are always 512-byte units, whatever the drive's sector size. */
const SECTOR = 512;

export const CONFIDENCE: Record<string, { label: string; tone: Tone }> = {
  confident: { label: "Confident", tone: "green" },
  rebuilt: { label: "Rebuilt", tone: "amber" },
  low_confidence: { label: "Low confidence", tone: "red" },
};
const CONFIDENCE_NOTE: Record<string, string> = {
  confident: "the partition table lists it and its filesystem checks out.",
  rebuilt: "missing from the partition table; SectorForge found it by recognising a filesystem on disk. Usually real, but its size may be unknown.",
  low_confidence:
    "the table lists it, but its first sector is not a filesystem SectorForge recognises. Normal for Microsoft reserved and other system partitions; otherwise it may be damaged or encrypted.",
};
const confidenceOf = (c: string) => CONFIDENCE[c] ?? { label: c, tone: "dim" as Tone };

export function PartitionsScreen() {
  const source = useApp((s) => s.source);
  const running = useApp((s) => s.running);
  const deep = useScan((s) => s.deep);
  const pct = useScan((s) => s.pct);
  const result = useScan((s) => s.result);
  const setSource = (v: string) => useApp.setState({ source: v });

  return (
    <>
      <h2>Find partitions</h2>
      <p className="lead">Read the partition table and, if needed, search the whole drive for filesystems. Nothing is written.</p>
      <ErrorAlert />
      <div className="cols">
        <div>
          <DrivePicker selected={source} onPick={setSource} />
          <Panel label="Scan settings">
            <Field
              id="psrc"
              label="Source"
              action={
                <Button
                  onClick={async () => {
                    const v = await dialogs.openFile("Choose an image file");
                    if (v !== null) setSource(v);
                  }}
                >
                  Browse
                </Button>
              }
            >
              <TextInput id="psrc" value={source} placeholder="\\.\PhysicalDrive1 or an image file" onChange={(e) => setSource(e.target.value)} />
            </Field>
            <Check checked={deep} onChange={(on) => useScan.setState({ deep: on })}>
              Deep scan (reads the entire drive, slow, finds partitions missing from the table)
            </Check>
            <div className="actions">
              <Button variant="primary" disabled={!!running} onClick={() => void startScan(source, deep)}>
                Scan
              </Button>
              <Button disabled={running !== "scan"} onClick={cancelScan}>
                Cancel
              </Button>
            </div>
            {running === "scan" && (
              <div className="spaced">
                <ProgressBar value={pct > 0 ? pct : null} label="Scan progress" />
                <JobActivity job="scan" />
              </div>
            )}
          </Panel>
        </div>
        <div>{result ? <Results result={result} /> : <ScanExplainer />}</div>
      </div>
    </>
  );
}

function ScanExplainer() {
  return (
    <Panel title="What a scan tells you">
      <ol className="steps">
        <li>
          <b>Quick scan.</b> Reads the partition table (MBR or GPT) at the start of the drive. Takes a second or two, even on a failing drive.
        </li>
        <li>
          <b>Deep scan.</b> Also reads every sector looking for the start of a filesystem (NTFS, FAT, exFAT, ext). Use it when a partition has
          disappeared, the table was wiped, or the drive was re-partitioned. It takes as long as reading the whole drive.
        </li>
        <li>
          <b>Confidence.</b> <Badge tone="green">Confident</Badge> the table lists it and its filesystem checks out.{" "}
          <Badge tone="amber">Rebuilt</Badge> found on disk but missing from the table. <Badge tone="red">Low confidence</Badge> the table
          lists it, but its first sector is not a filesystem SectorForge recognises (damaged, encrypted, or unsupported).
        </li>
        <li>
          <b>Next.</b> Open Recover; it offers the partitions found here for the same source.
        </li>
      </ol>
    </Panel>
  );
}

function Results({ result }: { result: ScanResult }) {
  const table = result.kind === "none" ? "no partition table" : `${result.kind.toUpperCase()} table`;
  return (
    <Panel title={`Partitions (${table})`}>
      <p className="dim small flush">
        Scanned <span className="mono">{result.source}</span>
      </p>
      {result.partitions.length === 0 ? (
        <p>No partitions were found.</p>
      ) : (
        <>
          <DiskLayout partitions={result.partitions} total={result.deviceSize} />
          <table>
            <thead>
              <tr>
                <th>Filesystem</th>
                <th>Start</th>
                <th>Size</th>
                <th>Confidence</th>
                <th />
              </tr>
            </thead>
            <tbody>
              {result.partitions.map((p) => {
                const c = confidenceOf(p.confidence);
                return (
                  <tr key={`${p.start_lba}-${p.filesystem}`}>
                    <td>
                      {p.filesystem.toUpperCase()} {p.label && <span className="dim">{p.label}</span>}
                    </td>
                    <td className="m nowrap" title={`${(p.start_lba * SECTOR).toLocaleString("en-US")} bytes, LBA ${p.start_lba}`}>
                      {fmtBytes(p.start_lba * SECTOR)}
                    </td>
                    <td className="m nowrap">{p.size_sectors === null ? "unknown" : fmtBytes(p.size_sectors * SECTOR)}</td>
                    <td className="nowrap">
                      <Badge tone={c.tone}>{c.label}</Badge>
                    </td>
                    <td className="nowrap">
                      {canBrowse(p) && (
                        <Button small onClick={() => recoverFrom(result.source, p.start_lba)}>
                          Recover files
                        </Button>
                      )}
                    </td>
                  </tr>
                );
              })}
            </tbody>
          </table>
          <ul className="notes">
            {[...new Set(result.partitions.map((p) => p.confidence))].map((k) => (
              <li key={k}>
                <b>{confidenceOf(k).label}:</b> {CONFIDENCE_NOTE[k] ?? ""}
              </li>
            ))}
          </ul>
          <p className="dim small flush">Hover a start position for the exact byte offset and LBA.</p>
        </>
      )}
    </Panel>
  );
}

function DiskLayout({ partitions, total }: { partitions: Partition[]; total: number }) {
  if (!total) return null;
  return (
    <>
      <div className="layout" role="img" aria-label={`Disk layout: ${partitions.length} partitions`}>
        {partitions.map((p) => {
          const left = ((p.start_lba * SECTOR) / total) * 100;
          const width = p.size_sectors === null ? 0 : ((p.size_sectors * SECTOR) / total) * 100;
          return (
            <div
              key={`${p.start_lba}-${p.filesystem}`}
              className={`tone-${confidenceOf(p.confidence).tone}`}
              style={{ left: `${left}%`, width: `${width}%` }}
              title={`${p.filesystem} at LBA ${p.start_lba}`}
            />
          );
        })}
      </div>
      <div className="layout-scale dim">
        <span className="mono">0</span>
        <span className="mono">{fmtBytes(total)}</span>
      </div>
    </>
  );
}

/** Opens Recover on this source with the partition picked, ready to list files. */
function recoverFrom(source: string, lba: number) {
  useRecover.setState({ source, sourceEdited: true, lba, tab: "files" });
  goTo("recover");
}
