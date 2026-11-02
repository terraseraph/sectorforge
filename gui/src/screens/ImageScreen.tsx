import { useEffect, useState } from "react";
import { DrivePicker } from "../components/DrivePicker";
import { HealthMap } from "../components/HealthMap";
import { ErrorAlert, JobActivity } from "../components/Shell";
import { Button, Check, Field, Hint, Notice, Panel, TextInput } from "../components/ui";
import { type Access, dialogs } from "../lib/api";
import { findDevice, mapPathFor, suggestImageName, systemDriveFor } from "../lib/devices";
import { mapChanged, pauseImage, startImage } from "../store/actions";
import { confirmKey, systemDriveBlocked, useApp } from "../store/app";
import { PRESETS, useImage } from "../store/image";

const setApp = useApp.setState;

export function ImageScreen() {
  const source = useApp((s) => s.source);
  const dest = useApp((s) => s.dest);
  const map = useApp((s) => s.map);
  const running = useApp((s) => s.running);
  const blocked = useApp(systemDriveBlocked);
  const hasMap = useImage((s) => s.hasMap);
  const mapDone = useImage((s) => s.mapDone);
  const resumable = hasMap && !mapDone;
  const retryOnly = useImage((s) => s.retryOnly);

  return (
    <>
      <h2>{retryOnly ? "Retry bad areas" : "Image a drive"}</h2>
      <p className="lead">
        {retryOnly
          ? "Runs only the second pass: goes back over the areas the map marks as damaged or skipped and reads them sector by sector. Use the same source, destination and map as the original run."
          : "Copy a failing drive to a safe file first, then work on the copy. Pause any time; running the same copy again picks up where it stopped."}
      </p>
      <ErrorAlert />
      <div className="cols">
        <div>
          <DrivePicker selected={source} onPick={(path) => setApp({ source: path })} />

          <Panel label="Image settings">
            <Field
              id="src"
              label="Source"
              action={<Button onClick={async () => pick(await dialogs.openFile("Choose a source image file"), "source")}>Browse</Button>}
            >
              <TextInput id="src" value={source} placeholder="\\.\PhysicalDrive1 or C:\path\disk.img" onChange={(e) => setApp({ source: e.target.value })} />
            </Field>
            <Hint>
              <code>\\.\PhysicalDrive1</code> = direct hardware access (needs Administrator). Any other path = image file.
            </Hint>
            <SystemDriveGuard />

            <Field
              id="dst"
              label="Destination"
              action={
                <Button
                  onClick={async () => {
                    const devices = useApp.getState().devices;
                    const need = findDevice(devices, source)?.size ?? undefined;
                    const v = await dialogs.saveFile("Destination image file", dest || suggestImageName(devices, source), need);
                    if (v !== null && setDest(v)) mapChanged();
                  }}
                >
                  Choose
                </Button>
              }
            >
              <TextInput id="dst" value={dest} onChange={(e) => setDest(e.target.value)} onBlur={() => map === mapPathFor(dest) && mapChanged()} />
            </Field>
            <Hint>Save the image on a different drive from the one you are rescuing. SectorForge refuses to write onto the source drive.</Hint>

            <Field
              id="map"
              label="Map file"
              action={
                <Button
                  onClick={async () => {
                    if (pick(await dialogs.saveFile("Map file", map || undefined), "map")) mapChanged();
                  }}
                >
                  Choose
                </Button>
              }
            >
              <TextInput id="map" value={map} onChange={(e) => setApp({ map: e.target.value })} onBlur={mapChanged} />
            </Field>
            <Hint>The map file records what has been copied. Keep it with the destination; it is what makes pause and resume work.</Hint>

            <Check checked={retryOnly} onChange={(on) => useImage.setState({ retryOnly: on })}>
              Retry bad areas only (skip the fast first pass)
            </Check>
            <AdvancedSettings />

            <div className="actions">
              <Button variant="primary" disabled={!!running || blocked} onClick={() => void startImage()}>
                {retryOnly ? "Retry bad areas" : resumable ? "Resume" : "Start"}
              </Button>
              <Button disabled={running !== "image"} onClick={pauseImage}>
                Pause
              </Button>
              {resumable && !running && <span className="dim">This map is part done; Resume carries on where it stopped.</span>}
            </div>
            <JobActivity job="image" />
          </Panel>
        </div>
        <div>
          <Tiles />
          <HealthMap />
          <Panel title="What happens when you press Start">
            <ol className="steps">
              <li>
                <b>Pass 1, fast copy.</b> Reads big chunks and jumps past anything slow or unreadable, so the healthy majority is safe first.
              </li>
              <li>
                <b>Pass 2, retry.</b> Goes back to the skipped areas and reads them sector by sector, up to the retry limit.
              </li>
              <li>
                <b>Done.</b> Anything still unreadable stays marked bad in the map. The image is still usable; Recover flags files that touch
                those areas.
              </li>
            </ol>
          </Panel>
        </div>
      </div>
    </>
  );
}

/** Sets the destination; the map follows it while the map is empty or still named after the old destination. Returns whether it followed. */
function setDest(dest: string): boolean {
  const s = useApp.getState();
  const follows = !s.map.trim() || s.map === mapPathFor(s.dest);
  setApp(follows && dest.trim() ? { dest, map: mapPathFor(dest) } : { dest });
  return follows;
}

function pick(value: string | null, field: "source" | "dest" | "map"): boolean {
  if (value === null) return false;
  setApp({ [field]: value });
  return true;
}

/** Imaging the drive the OS runs from needs a deliberate, explicit yes. */
function SystemDriveGuard() {
  const source = useApp((s) => s.source);
  const devices = useApp((s) => s.devices);
  const confirmed = useApp((s) => s.systemConfirmed);
  if (!systemDriveFor(devices, source)) return null;
  const key = confirmKey(source);
  return (
    <Notice tone="danger">
      <p>
        <b>This is your system drive.</b> It holds the operating system SectorForge is running on, so it is in constant use and cannot be
        copied cleanly while you are using it. A failing drive you are rescuing is normally a different one.
      </p>
      <label className="check">
        <input type="checkbox" checked={confirmed === key} onChange={(e) => setApp({ systemConfirmed: e.target.checked ? key : "" })} />I
        understand, and I want to image the system drive anyway
      </label>
    </Notice>
  );
}

function AdvancedSettings() {
  const chunkBytes = useImage((s) => s.chunkBytes);
  const skipBytes = useImage((s) => s.skipBytes);
  const retries = useImage((s) => s.retries);
  const access = useImage((s) => s.access);
  const timeoutSecs = useImage((s) => s.timeoutSecs);
  const reverse = useImage((s) => s.reverse);
  const set = useImage.setState;
  const active = PRESETS.find((p) => p.s.chunkBytes === chunkBytes && p.s.skipBytes === skipBytes && p.s.retries === retries && p.s.timeoutSecs === timeoutSecs);
  return (
    <details>
      <summary>Advanced</summary>
      <div className="presets" role="radiogroup" aria-label="Presets">
        {PRESETS.map((p) => (
          <button key={p.id} type="button" role="radio" aria-checked={active?.id === p.id} className="preset" onClick={() => set(p.s)}>
            {p.label}
          </button>
        ))}
      </div>
      <Hint flush>{active ? active.hint : "Custom settings."}</Hint>
      <div className="adv">
        <div>
          <label htmlFor="chunk">Read size (fast pass)</label>
          <SizeInput id="chunk" bytes={chunkBytes} onChange={(b) => set({ chunkBytes: b })} />
        </div>
        <div>
          <label htmlFor="skip">Jump past a bad read</label>
          <SizeInput id="skip" bytes={skipBytes} onChange={(b) => set({ skipBytes: b })} />
        </div>
        <div>
          <label htmlFor="retries">Retries per sector</label>
          <input id="retries" type="number" min={0} value={retries} onChange={(e) => set({ retries: e.target.valueAsNumber })} />
        </div>
      </div>
      <Hint flush>A bigger jump gets more data off a badly damaged drive sooner; the retry pass comes back for what was skipped.</Hint>
      <div className="adv">
        <div>
          <label htmlFor="access">Read the drive through</label>
          <select id="access" value={access} onChange={(e) => set({ access: e.target.value as Access })}>
            <option value="os">Windows / OS (default)</option>
            <option value="scsi">SCSI pass-through</option>
            <option value="ata">ATA pass-through</option>
          </select>
        </div>
        <div>
          <label htmlFor="timeout">Read timeout (seconds)</label>
          <input
            id="timeout"
            type="number"
            min={1}
            max={3600}
            disabled={access === "os"}
            value={timeoutSecs}
            onChange={(e) => set({ timeoutSecs: e.target.valueAsNumber })}
          />
        </div>
      </div>
      <Hint flush>{ACCESS_HINT[access]}</Hint>
      <Check checked={reverse} onChange={(on) => set({ reverse: on })}>
        Read backwards (fast pass goes from the end of the drive to the start)
      </Check>
    </details>
  );
}

const UNITS = [
  { label: "bytes", size: 1 },
  { label: "sectors (512 B)", size: 512 },
  { label: "KiB", size: 1024 },
  { label: "MiB", size: 1024 * 1024 },
] as const;

/** The biggest unit `bytes` is a whole number of. */
const unitFor = (bytes: number) => [...UNITS].reverse().find((u) => bytes >= u.size && bytes % u.size === 0)?.size ?? 1;

/** A size in bytes, typed in the unit picked beside it. */
function SizeInput({ id, bytes, onChange }: { id: string; bytes: number; onChange: (bytes: number) => void }) {
  const [unit, setUnit] = useState(() => unitFor(bytes));
  // A preset can set a size the current unit can't show as a whole number.
  useEffect(() => {
    if (Number.isFinite(bytes) && bytes % unit !== 0) setUnit(unitFor(bytes));
  }, [bytes, unit]);
  return (
    <div className="size-input">
      <input
        id={id}
        type="number"
        min={1}
        value={Number.isFinite(bytes) ? bytes / unit : ""}
        onChange={(e) => onChange(Math.round(e.target.valueAsNumber * unit))}
      />
      <select aria-label="Unit" value={unit} onChange={(e) => setUnit(Number(e.target.value))}>
        {UNITS.map((u) => (
          <option key={u.size} value={u.size}>
            {u.label}
          </option>
        ))}
      </select>
    </div>
  );
}

const ACCESS_HINT: Record<Access, string> = {
  os: "Reads go through the operating system's disk driver. Works with every drive and image file, but on a bad sector the driver can retry for minutes before giving up.",
  scsi: "Sends SCSI READ commands straight to the drive with your timeout, so a bad sector costs seconds, not minutes. Works with most SATA, SAS and USB drives. Image files ignore this.",
  ata: "Sends ATA READ commands straight to the drive with your timeout. For SATA drives on a SATA port; NVMe drives and most USB enclosures don't support it. Image files ignore this.",
};

function Tiles() {
  const tiles = useImage((s) => s.tiles);
  const pass = useImage((s) => s.pass);
  const items = [
    ["Rescued", tiles.rescued],
    ["Bad", tiles.bad],
    ["Pending", tiles.pending],
    ["Current pass", pass],
  ] as const;
  return (
    <div className="tiles">
      {items.map(([k, v]) => (
        <div key={k} className="tile">
          <div className="k">{k}</div>
          <div className="v">{v}</div>
        </div>
      ))}
    </div>
  );
}
