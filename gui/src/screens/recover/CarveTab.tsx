import { useEffect } from "react";
import { PartitionPicker } from "../../components/PartitionPicker";
import { VirtualList } from "../../components/VirtualList";
import { JobActivity } from "../../components/Shell";
import { Badge, Busy, Button, Field, Hint, Panel, ProgressBar, TextInput } from "../../components/ui";
import { dialogs } from "../../lib/api";
import { fmtBytes, fmtCount, percent } from "../../lib/format";
import { loadTypes, startCarve, startScan, stopCarve } from "../../store/actions";
import { useApp } from "../../store/app";
import { type CarvedRow, useRecover } from "../../store/recover";
import { partitionsFor, useScan } from "../../store/scan";

export function CarveTab({ source }: { source: string }) {
  const running = useApp((s) => s.running);
  const types = useRecover((s) => s.types);
  const typesError = useRecover((s) => s.typesError);
  const carvePart = useRecover((s) => s.carvePart);
  const carveOut = useRecover((s) => s.carveOut);
  const result = useScan((s) => s.result);
  const parts = partitionsFor(result, source);

  useEffect(() => {
    if (!types && !typesError) void loadTypes();
  }, [types, typesError]);

  return (
    <>
      <Panel label="Carve settings">
        <p className="lead flush">
          Carving looks for known file types by their first bytes, so it works even with no working filesystem. Recovered files have no
          names or folders.
        </p>
        <h3>File types to look for</h3>
        <TypePicker />

        <div className="spaced">
          <div className="panel-head">
            <h3>Search in</h3>
            <Button small disabled={!!running || !source} onClick={() => void startScan(source, false)}>
              Rescan
            </Button>
          </div>
          <PartitionPicker
            id="carve"
            parts={parts}
            size={result?.deviceSize ?? null}
            source={source}
            selected={carvePart === "" ? null : Number(carvePart)}
            onSelect={(v) => useRecover.setState({ carvePart: v === null ? "" : String(v) })}
            whole="The whole source"
          />
        </div>
        <Hint flush>Limiting the search to one partition is faster and avoids matches from other volumes.</Hint>

        <Field
          id="cout"
          label="Save to"
          action={
            <Button
              onClick={async () => {
                const d = await dialogs.openFolder("Choose where to save carved files");
                if (d !== null) useRecover.setState({ carveOut: d });
              }}
            >
              Choose
            </Button>
          }
        >
          <TextInput
            id="cout"
            value={carveOut}
            placeholder="A folder on a different drive from the one you are rescuing"
            onChange={(e) => useRecover.setState({ carveOut: e.target.value })}
          />
        </Field>
        <div className="actions">
          <Button variant="primary" disabled={!!running || !types} onClick={() => void startCarve()}>
            Start carving
          </Button>
          <Button disabled={running !== "carve"} onClick={stopCarve}>
            Stop
          </Button>
        </div>
        <CarveProgress />
      </Panel>
      <CarveResults />
    </>
  );
}

function TypePicker() {
  const types = useRecover((s) => s.types);
  const typesError = useRecover((s) => s.typesError);
  const chosen = useRecover((s) => s.chosen);

  if (typesError) return <p className="dim">Could not load the list of file types: {typesError}</p>;
  if (!types) return <Busy>Loading the list of file types SectorForge can find...</Busy>;

  const toggle = (id: string, on: boolean) =>
    useRecover.setState((s) => ({ chosen: on ? [...s.chosen.filter((c) => c !== id), id] : s.chosen.filter((c) => c !== id) }));
  return (
    <>
      <div className="types">
        {types.map((t) => (
          <label key={t.id} className="check">
            <input type="checkbox" checked={chosen.includes(t.id)} onChange={(e) => toggle(t.id, e.target.checked)} />
            <b>{t.id.toUpperCase()}</b> <span className="dim">{t.description}</span>
          </label>
        ))}
      </div>
      <div className="actions tight">
        <Button small onClick={() => useRecover.setState({ chosen: types.map((t) => t.id) })}>
          Select all
        </Button>
        <Button small onClick={() => useRecover.setState({ chosen: [] })}>
          Select none
        </Button>
      </div>
    </>
  );
}

function CarveProgress() {
  const c = useRecover((s) => s.carve);
  if (c.state === "running") {
    return (
      <div className="spaced">
        <ProgressBar value={c.total ? percent(c.scanned, c.total) : null} label="Carving progress" />
        <JobActivity job="carve" />
      </div>
    );
  }
  if (!c.summary) return null;
  return <Hint flush>{c.summary}</Hint>;
}

function CarveResults() {
  const c = useRecover((s) => s.carve);
  if (c.state === "") return null;
  return (
    <Panel title="Files found">
      <div className="frow fhead cv">
        <span>Type</span>
        <span>Saved as</span>
        <span>Size</span>
        <span>Position on source</span>
        <span>State</span>
      </div>
      <VirtualList count={c.rows.length} rowHeight={30} label="Files found" empty="No files found yet." renderRow={(k) => <Row row={c.rows[k]!} />} />
    </Panel>
  );
}

function Row({ row }: { row: CarvedRow }) {
  return (
    <div className="frow cv">
      <span>
        <Badge tone="accent">{row.type.toUpperCase()}</Badge>
      </span>
      <span className="nm">{row.name}</span>
      <span className="m">{fmtBytes(row.size)}</span>
      <span className="m dim">{fmtCount(row.offset)} B</span>
      <span>{row.complete ? <Badge tone="green">Complete</Badge> : <Badge tone="amber">Incomplete</Badge>}</span>
    </div>
  );
}
