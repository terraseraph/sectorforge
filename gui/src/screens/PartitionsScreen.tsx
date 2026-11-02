import { DrivePicker } from "../components/DrivePicker";
import { ErrorAlert } from "../components/Shell";
import { Badge, Button, Check, Field, Panel, ProgressBar, TextInput, type Tone } from "../components/ui";
import type { Partition } from "../lib/contract";
import { fmtBytes } from "../lib/format";
import { cancelScan, startScan } from "../store/actions";
import { useApp } from "../store/app";
import { type ScanResult, useScan } from "../store/scan";

/** `*_lba` / `*_sectors` are always 512-byte units, whatever the drive's sector size. */
const SECTOR = 512;

export const CONFIDENCE: Record<string, { label: string; tone: Tone }> = {
  confident: { label: "Confident", tone: "green" },
  rebuilt: { label: "Rebuilt", tone: "amber" },
  low_confidence: { label: "Low confidence", tone: "red" },
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
      <DrivePicker selected={source} onPick={setSource} />
      <Panel label="Scan settings">
        <Field id="psrc" label="Source">
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
        {running === "scan" && pct > 0 && (
          <div className="spaced">
            <ProgressBar value={pct} label="Scan progress" />
          </div>
        )}
      </Panel>
      {result && <Results result={result} />}
    </>
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
                    <td className="m">
                      {(p.start_lba * SECTOR).toLocaleString("en-US")} B <span className="dim">(LBA {p.start_lba})</span>
                    </td>
                    <td className="m">{p.size_sectors === null ? "unknown" : fmtBytes(p.size_sectors * SECTOR)}</td>
                    <td>
                      <Badge tone={c.tone}>{c.label}</Badge>
                    </td>
                  </tr>
                );
              })}
            </tbody>
          </table>
          <p className="dim flush">
            <b>Rebuilt</b> means the partition is missing from the drive's partition table, so SectorForge found it by recognising a
            filesystem on disk. It is usually real, but its exact size may not be known.
          </p>
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
