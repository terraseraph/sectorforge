import { VirtualList } from "../../components/VirtualList";
import { JobActivity } from "../../components/Shell";
import { Badge, Hint, Panel, ProgressBar } from "../../components/ui";
import { fmtBytes, fmtCount, percent } from "../../lib/format";
import { type ExtractedRow, useRecover } from "../../store/recover";

export function ExtractPanel() {
  const x = useRecover((s) => s.extract);
  if (x.state === "") return null;
  const progress =
    x.summary ||
    `${fmtCount(x.filesDone)} of ${fmtCount(x.filesTotal)} files, ${fmtBytes(x.bytesDone)}${x.bytesTotal ? ` of ${fmtBytes(x.bytesTotal)}` : ""}.`;

  return (
    <Panel title="Extraction">
      <ProgressBar value={x.filesTotal ? percent(x.filesDone, x.filesTotal) : x.state === "running" ? null : 100} label="Extraction progress" />
      <JobActivity job="extract" />
      <Hint flush>{progress}</Hint>
      <VirtualList short count={x.rows.length} rowHeight={30} label="Extracted files" empty="Waiting for the first file..." renderRow={(k) => <Row row={x.rows[k]!} />} />
    </Panel>
  );
}

function Row({ row }: { row: ExtractedRow }) {
  return (
    <div className="xrow">
      <span className="nm" title={row.note ?? undefined}>
        {row.path ?? "(not written)"}
      </span>
      <span className="m">{row.path === null ? "" : fmtBytes(row.size)}</span>
      <span>
        {row.path === null ? (
          <Badge tone="red">Failed</Badge>
        ) : row.complete ? (
          <Badge tone="green">Complete</Badge>
        ) : (
          <Badge tone="amber">Incomplete</Badge>
        )}
      </span>
      <span className="dim note">{row.note ?? ""}</span>
    </div>
  );
}
