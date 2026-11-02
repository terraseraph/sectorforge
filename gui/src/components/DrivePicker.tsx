import { useState } from "react";
import { busLabel, deviceTitle, findDevice } from "../lib/devices";
import { fmtBytes, isDevicePath } from "../lib/format";
import { loadDevices, restartAsAdmin } from "../store/actions";
import { useApp } from "../store/app";
import { Badge, Busy, Button, Notice, Panel } from "./ui";

/** Drives from `sf-cli devices`; picking one sets `selected` via `onPick`. */
export function DrivePicker({ selected, onPick }: { selected: string; onPick: (path: string) => void }) {
  const devices = useApp((s) => s.devices);
  const state = useApp((s) => s.devicesState);
  const error = useApp((s) => s.devicesError);
  const current = findDevice(devices, selected)?.path;

  let body;
  if (state === "loading" && !devices.length) body = <Busy>Asking the system which drives are attached...</Busy>;
  else if (state === "error") body = <p className="dim">Could not list drives: {error}</p>;
  else if (!devices.length) body = <p className="dim">No drives were found. You can still type an image file path below.</p>;
  else
    body = (
      <div className="devs" role="radiogroup" aria-label="Drives found">
        {devices.map((d) => (
          <button key={d.path} type="button" className="dev" role="radio" aria-checked={d.path === current} onClick={() => onPick(d.path)}>
            <span className="dt">
              {deviceTitle(d)} {d.system && <Badge tone="red">System drive</Badge>} {d.removable && <Badge tone="dim">Removable</Badge>}
            </span>
            <span className="dm">
              {fmtBytes(d.size)}, {busLabel(d.bus)} · <span className="mono">{d.path}</span>
            </span>
          </button>
        ))}
      </div>
    );

  return (
    <Panel
      title="Drives"
      actions={
        <Button small disabled={state === "loading"} onClick={() => void loadDevices()}>
          {state === "loading" ? "Refreshing..." : "Refresh"}
        </Button>
      }
    >
      {body}
      <ElevationNotice selected={selected} />
    </Panel>
  );
}

/** Explains what reading a drive needs on this OS, when we don't have it yet. */
function ElevationNotice({ selected }: { selected: string }) {
  const elevated = useApp((s) => s.elevated);
  const cli = useApp((s) => s.cli);
  const [message, setMessage] = useState("");
  if (!isDevicePath(selected.trim()) || elevated !== false) return null;

  const windows = (cli?.platform || (navigator.userAgent.includes("Windows") ? "windows" : "linux")) === "windows";
  if (windows) {
    return (
      <Notice tone="warn">
        <p>
          <b>Administrator rights needed.</b> Windows only lets Administrators read a drive directly. SectorForge will close and reopen as
          Administrator with these settings kept. Windows will ask you to confirm.
        </p>
        <div className="actions">
          <Button variant="primary" onClick={async () => setMessage((await restartAsAdmin()) ?? "")}>
            Restart as Administrator
          </Button>
          {message && <span className="dim">{message}</span>}
        </div>
      </Notice>
    );
  }
  if (cli && !cli.pkexec) {
    return (
      <Notice tone="warn">
        <p>
          <b>Administrator rights needed, and pkexec was not found.</b> Reading a drive directly needs root. Install polkit (the pkexec
          command), start SectorForge with sudo, or add your user to the disk group. Or work from an image file instead.
        </p>
      </Notice>
    );
  }
  return (
    <Notice>
      <p>
        <b>Administrator rights needed.</b> When you start, your system will ask for your password (through pkexec) so SectorForge can read
        the drive.
      </p>
    </Notice>
  );
}
