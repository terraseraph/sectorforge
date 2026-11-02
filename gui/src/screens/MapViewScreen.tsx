import { useState } from "react";
import { HealthMap } from "../components/HealthMap";
import { ErrorAlert } from "../components/Shell";
import { Busy, Button, Field, Hint, Panel, TextInput } from "../components/ui";
import { api, dialogs, errorText } from "../lib/api";
import type { EvOf } from "../lib/api";
import { fmtBytes } from "../lib/format";
import { goTo, mapChanged, showError } from "../store/actions";
import { useApp } from "../store/app";
import { tilesFromTotals, useImage } from "../store/image";

export function MapViewScreen() {
  const current = useApp((s) => s.map);
  const [path, setPath] = useState(current);
  const [loading, setLoading] = useState(false);
  const [status, setStatus] = useState<(EvOf<"status"> & { path: string }) | null>(null);

  const load = async (p = path) => {
    const map = p.trim();
    if (!map) return showError("Choose a map file first.");
    setLoading(true);
    try {
      setStatus({ ...(await api.getStatus(map)), path: map });
      useApp.setState({ error: "" });
    } catch (e) {
      setStatus(null);
      showError(errorText(e));
    } finally {
      setLoading(false);
    }
  };

  // Hand the map to the Image screen, either to carry on or to retry only its bad areas.
  const useMap = (retryOnly: boolean) => {
    if (!status) return;
    useApp.setState({ map: status.path });
    mapChanged();
    useImage.setState({ retryOnly });
    goTo("image");
  };

  const t = status ? tilesFromTotals(status.size, status.totals) : null;
  return (
    <>
      <h2>Map viewer</h2>
      <p className="lead">
        A map file is the record an imaging run keeps of every area it has tried. Open one to see how a rescue went, without reading any drive.
      </p>
      <ErrorAlert />
      <div className="cols">
        <div>
          <Panel label="Map file">
            <Field
              id="mvpath"
              label="Map file"
              action={
                <Button
                  onClick={async () => {
                    const v = await dialogs.openFile("Choose a map file");
                    if (v !== null) {
                      setPath(v);
                      void load(v);
                    }
                  }}
                >
                  Browse
                </Button>
              }
            >
              <TextInput id="mvpath" value={path} placeholder="E:\rescue\disk.map" onChange={(e) => setPath(e.target.value)} />
            </Field>
            <Hint>Only the map is read. The image and the drive are not touched.</Hint>
            <div className="actions">
              <Button variant="primary" disabled={loading} onClick={() => void load()}>
                Open map
              </Button>
              {loading && <Busy>Reading {path.trim()}...</Busy>}
            </div>
          </Panel>
          {status && t && (
            <Panel title="Summary">
              <div className="tiles">
                <Tile k="Drive size" v={fmtBytes(status.size)} />
                <Tile k="Rescued" v={t.rescued} />
                <Tile k="Bad" v={t.bad} />
                <Tile k="Not yet read" v={t.pending} />
              </div>
              <p className="dim small flush">
                {status.totals.bad_sector || status.totals.non_scraped || status.totals.non_trimmed
                  ? "Some areas are still damaged. Retrying them reads only those areas, not the whole drive again."
                  : status.totals.non_tried
                    ? "Part of the drive has not been read yet. Resuming carries on where the run stopped."
                    : "Every area was read. Nothing is left to retry."}
              </p>
              <div className="actions">
                <Button onClick={() => useMap(false)}>Resume imaging with this map</Button>
                <Button onClick={() => useMap(true)}>Retry bad areas</Button>
              </div>
            </Panel>
          )}
        </div>
        <div>
          <HealthMap snapshot={status ? { size: status.size, blocks: status.blocks } : null} />
        </div>
      </div>
    </>
  );
}

function Tile({ k, v }: { k: string; v: string }) {
  return (
    <div className="tile">
      <div className="k">{k}</div>
      <div className="v">{v}</div>
    </div>
  );
}
