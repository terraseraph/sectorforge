import { motion } from "motion/react";
import { ErrorAlert } from "../components/Shell";
import { Badge, Busy, Button, Panel } from "../components/ui";
import type { Device } from "../lib/contract";
import { busLabel, deviceTitle, findDevice } from "../lib/devices";
import { fmtBytes } from "../lib/format";
import { goTo, loadDevices, startScan } from "../store/actions";
import { useApp } from "../store/app";
import { useImage } from "../store/image";
import { useVerify } from "../store/verify";

export function DrivesScreen() {
  const devices = useApp((s) => s.devices);
  const state = useApp((s) => s.devicesState);
  const error = useApp((s) => s.devicesError);
  const elevated = useApp((s) => s.elevated);
  const picked = useApp((s) => s.focusDrive);
  const setPicked = (focusDrive: string) => useApp.setState({ focusDrive });
  const d = findDevice(devices, picked) ?? devices[0];

  return (
    <>
      <h2>Drive details</h2>
      <p className="lead">
        Every physical drive the system reports. Check the model and serial number before you start: they are the surest way to tell drives
        apart, since drive numbers can change after a restart or re-plug.
      </p>
      <ErrorAlert />
      <div className="cols">
        <div>
          <Panel
            title="Attached drives"
            actions={
              <Button small disabled={state === "loading"} onClick={() => void loadDevices()}>
                {state === "loading" ? "Refreshing..." : "Refresh"}
              </Button>
            }
          >
            {state === "loading" && !devices.length ? (
              <Busy>Asking the system which drives are attached...</Busy>
            ) : state === "error" ? (
              <p className="dim">Could not list drives: {error}</p>
            ) : !devices.length ? (
              <p className="dim">No drives were found.</p>
            ) : (
              <div className="drive-list" role="listbox" aria-label="Drives">
                {devices.map((x) => (
                  <button key={x.path} type="button" role="option" aria-selected={x === d} className="drive-row" onClick={() => setPicked(x.path)}>
                    {x === d && <motion.span layoutId="drive-sel" className="drive-sel" transition={{ type: "spring", bounce: 0, visualDuration: 0.25 }} />}
                    <span className="dt">
                      {deviceTitle(x)} {x.system && <Badge tone="red">System</Badge>} {x.removable && <Badge tone="dim">Removable</Badge>}
                    </span>
                    <span className="dm">
                      {fmtBytes(x.size)}, {busLabel(x.bus)} · <span className="mono">{x.path}</span>
                    </span>
                  </button>
                ))}
              </div>
            )}
            <p className="dim small flush">
              {elevated === null
                ? ""
                : elevated
                  ? "Running with Administrator rights: drives can be read directly."
                  : "Not running as Administrator: drives are listed, but reading one directly will ask for permission."}
            </p>
          </Panel>
        </div>
        <div>{d && <Details d={d} />}</div>
      </div>
    </>
  );
}

function Details({ d }: { d: Device }) {
  const running = useApp((s) => s.running);
  const rows: [string, React.ReactNode][] = [
    ["Model", deviceTitle(d)],
    ["Serial number", d.serial?.trim() || <span className="dim">not reported</span>],
    ["Path", <span className="mono">{d.path}</span>],
    ["Size", d.size === null ? "unknown" : `${fmtBytes(d.size)} (${d.size.toLocaleString("en-US")} bytes)`],
    ["Sector size", d.sector_size ? `${d.sector_size} bytes` : "unknown"],
    ["Connection", busLabel(d.bus)],
    ["Removable", d.removable ? "Yes" : "No"],
    ["System drive", d.system ? "Yes. Windows or Linux is running from it." : "No"],
  ];
  return (
    <motion.div key={d.path} initial={{ opacity: 0, x: 8 }} animate={{ opacity: 1, x: 0 }} transition={{ type: "spring", bounce: 0, visualDuration: 0.25 }}>
      <Panel title={deviceTitle(d)}>
        <table className="kv">
          <tbody>
            {rows.map(([k, v]) => (
              <tr key={k}>
                <th>{k}</th>
                <td>{v}</td>
              </tr>
            ))}
          </tbody>
        </table>
        <h3>Use this drive</h3>
        <div className="actions">
          <Button
            variant="primary"
            onClick={() => {
              useApp.setState({ source: d.path });
              useImage.setState({ retryOnly: false });
              goTo("image");
            }}
          >
            Image it
          </Button>
          <Button
            disabled={!!running}
            onClick={() => {
              useApp.setState({ source: d.path });
              goTo("partitions");
              void startScan(d.path, false);
            }}
          >
            Scan partitions
          </Button>
          <Button
            onClick={() => {
              useVerify.setState({ source: d.path });
              goTo("verify");
            }}
          >
            Hash it
          </Button>
        </div>
        {d.system && <p className="dim small flush">This is the drive your system runs from. It changes constantly, so an image or hash of it is never quite stable.</p>}
      </Panel>
    </motion.div>
  );
}
