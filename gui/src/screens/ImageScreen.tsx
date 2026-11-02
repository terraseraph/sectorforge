import { DrivePicker } from "../components/DrivePicker";
import { HealthMap } from "../components/HealthMap";
import { ErrorAlert } from "../components/Shell";
import { Button, Field, Hint, Notice, Panel, TextInput } from "../components/ui";
import { dialogs } from "../lib/api";
import { systemDriveFor } from "../lib/devices";
import { mapChanged, pauseImage, startImage } from "../store/actions";
import { confirmKey, systemDriveBlocked, useApp } from "../store/app";
import { useImage } from "../store/image";

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

  return (
    <>
      <h2>Image a drive</h2>
      <p className="lead">
        Copy a failing drive to a safe file first, then work on the copy. Pause any time; running the same copy again picks up where it
        stopped.
      </p>
      <ErrorAlert />
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

        <Field id="dst" label="Destination" action={<Button onClick={async () => pick(await dialogs.saveFile("Destination image file"), "dest")}>Choose</Button>}>
          <TextInput id="dst" value={dest} onChange={(e) => setApp({ dest: e.target.value })} />
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

        <AdvancedSettings />

        <div className="actions">
          <Button variant="primary" disabled={!!running || blocked} onClick={() => void startImage()}>
            {resumable ? "Resume" : "Start"}
          </Button>
          <Button disabled={running !== "image"} onClick={pauseImage}>
            Pause
          </Button>
          {resumable && !running && <span className="dim">This map is part done; Resume carries on where it stopped.</span>}
        </div>
      </Panel>

      <Tiles />
      <HealthMap />
    </>
  );
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
  const chunkKiB = useImage((s) => s.chunkKiB);
  const skipKiB = useImage((s) => s.skipKiB);
  const retries = useImage((s) => s.retries);
  const set = useImage.setState;
  return (
    <details>
      <summary>Advanced</summary>
      <div className="adv">
        <div>
          <label htmlFor="chunk">Chunk size (KiB)</label>
          <input id="chunk" type="number" min={1} value={chunkKiB} onChange={(e) => set({ chunkKiB: e.target.valueAsNumber })} />
        </div>
        <div>
          <label htmlFor="skip">Skip size (KiB)</label>
          <input id="skip" type="number" min={1} value={skipKiB} onChange={(e) => set({ skipKiB: e.target.valueAsNumber })} />
        </div>
        <div>
          <label htmlFor="retries">Retries per sector</label>
          <input id="retries" type="number" min={0} value={retries} onChange={(e) => set({ retries: e.target.valueAsNumber })} />
        </div>
      </div>
      <Hint flush>Defaults suit most drives. A bigger skip size gets more data off a badly damaged drive sooner.</Hint>
    </details>
  );
}

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
