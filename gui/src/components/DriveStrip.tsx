// Home's "This PC": every attached drive (and any scanned image file) with its size, partitions
// drawn to scale, and what SectorForge knows about it. Clicking one opens the inspector.
import { AlertTriangle, Disc3, HardDrive, type LucideIcon, MemoryStick, Usb } from "lucide-react";
import { motion } from "motion/react";
import { useCallback, useState } from "react";
import type { Device } from "../lib/contract";
import { busLabel, deviceTitle } from "../lib/devices";
import { fmtBytes, isDevicePath } from "../lib/format";
import { useApp } from "../store/app";
import { useImage } from "../store/image";
import { useScan } from "../store/scan";
import { useVerify } from "../store/verify";
import { DriveInspector, type InspectItem } from "./DriveInspector";
import { PartitionMap } from "./PartitionMap";
import { Badge, Busy, cx } from "./ui";

interface Item extends InspectItem {
  meta: string;
}

const norm = (p: string) => p.trim().toLowerCase();

function iconFor(d: Device): LucideIcon {
  if (d.bus === "usb") return Usb;
  if (d.bus === "nvme" || d.bus === "sd") return MemoryStick;
  if (d.bus === "virtual") return Disc3;
  return HardDrive;
}

export function DriveStrip() {
  const devices = useApp((s) => s.devices);
  const state = useApp((s) => s.devicesState);
  const scanned = useScan((s) => s.bySource);
  const [inspect, setInspect] = useState<Item | null>(null);
  const close = useCallback(() => setInspect(null), []);

  const items: Item[] = devices.map((d) => ({
    path: d.path,
    title: deviceTitle(d),
    size: d.size,
    meta: `${busLabel(d.bus)} · ${d.path}`,
    icon: iconFor(d),
    device: d,
  }));
  // Image files that were scanned have partitions worth showing next to the drives.
  for (const r of Object.values(scanned)) {
    if (isDevicePath(r.source) || items.some((i) => norm(i.path) === norm(r.source))) continue;
    const name = r.source.split(/[\\/]/).pop() || r.source;
    items.push({ path: r.source, title: name, size: r.deviceSize || null, meta: `Image file · ${r.source}`, icon: Disc3, device: null });
  }

  return (
    <section className="drives-strip" aria-label="Drives">
      <div className="strip-head">
        <h3>Drives on this computer</h3>
        <Stats />
        <span className="dim small">Click a drive to inspect it.</span>
      </div>
      {state === "loading" && !items.length ? (
        <Busy>Asking the system which drives are attached...</Busy>
      ) : !items.length ? (
        <p className="dim small">{state === "error" ? "Could not list drives." : "No drives found."} Image files can still be opened from each tool.</p>
      ) : (
        <div className="drive-tiles">
          {items.map((it, i) => (
            <DriveTile key={it.path} item={it} index={i} active={inspect?.path === it.path} onOpen={() => setInspect(it)} />
          ))}
        </div>
      )}
      <DriveInspector item={inspect} onClose={close} />
    </section>
  );
}

/** Cheap totals from the drive list and the startup survey. */
function Stats() {
  const devices = useApp((s) => s.devices);
  const elevated = useApp((s) => s.elevated);
  const bySource = useScan((s) => s.bySource);
  const errors = useScan((s) => s.errors);
  const surveying = useScan((s) => s.surveying.length);
  if (!devices.length) return null;
  const keys = devices.map((d) => norm(d.path));
  const total = devices.reduce((n, d) => n + (d.size ?? 0), 0);
  const parts = keys.reduce((n, k) => n + (bySource[k]?.partitions.length ?? 0), 0);
  const bad = keys.filter((k) => errors[k]).length;
  return (
    <span className="strip-stats">
      <span>
        <b>{devices.length}</b> drives
      </span>
      <span>
        <b>{fmtBytes(total)}</b> total
      </span>
      {surveying > 0 ? (
        <span className="live">
          <motion.i className="dot" animate={{ opacity: [1, 0.25, 1] }} transition={{ duration: 1.2, repeat: Infinity }} aria-hidden="true" />
          reading {surveying} partition table{surveying === 1 ? "" : "s"}
        </span>
      ) : elevated === false ? (
        <span>partitions shown after a scan (quick scan at startup needs Administrator)</span>
      ) : (
        <span>
          <b>{parts}</b> partitions
        </span>
      )}
      {bad > 0 && (
        <span className="bad">
          <AlertTriangle size={12} aria-hidden="true" /> <b>{bad}</b> with read errors
        </span>
      )}
    </span>
  );
}

function DriveTile({ item, index, active, onOpen }: { item: Item; index: number; active: boolean; onOpen: () => void }) {
  const scan = useScan((s) => s.bySource[norm(item.path)]);
  const readError = useScan((s) => s.errors[norm(item.path)]);
  const status = useDriveStatus(item.path);
  const parts = scan?.partitions ?? null;
  const Icon = item.icon;
  return (
    <motion.button
      type="button"
      className={cx("drive-tile", active && "open", status.busy && "busy-card", readError && "has-error")}
      aria-haspopup="dialog"
      initial={{ opacity: 0, y: 8 }}
      animate={{ opacity: 1, y: 0 }}
      whileHover={{ y: -2 }}
      whileTap={{ scale: 0.985 }}
      transition={{ type: "spring", bounce: 0, visualDuration: 0.3, delay: index * 0.04 }}
      onClick={onOpen}
    >
      <span className="drive-top">
        <span className="drive-icon">
          <Icon size={24} strokeWidth={1.6} aria-hidden="true" />
        </span>
        <span className="drive-body">
          <span className="drive-name">
            <b>{item.title}</b>
            {item.device?.system && <Badge tone="red">System</Badge>}
            {item.device?.removable && <Badge tone="dim">Removable</Badge>}
          </span>
          <span className="drive-meta">
            <b>{fmtBytes(item.size)}</b> · <span className="mono">{item.meta}</span>
          </span>
        </span>
      </span>
      <PartitionMap parts={parts} size={item.size} />
      {readError && !status.busy ? (
        <span className="drive-status bad" title={readError}>
          <AlertTriangle size={13} aria-hidden="true" /> Read error. If it is failing, image it before anything else.
        </span>
      ) : (
        <span className={cx("drive-status", status.busy && "live")}>
          {status.busy && (
            <motion.i className="dot" animate={{ opacity: [1, 0.25, 1] }} transition={{ duration: 1.2, repeat: Infinity }} aria-hidden="true" />
          )}
          {status.text || (parts ? `${parts.length} partition${parts.length === 1 ? "" : "s"} · ${scan?.kind === "none" ? "no table" : scan?.kind.toUpperCase()}` : "Not scanned yet")}
        </span>
      )}
    </motion.button>
  );
}

/** What SectorForge is doing with, or knows about, this path. */
function useDriveStatus(path: string): { text: string; busy: boolean } {
  const key = norm(path);
  const running = useApp((s) => s.running);
  const imageSrc = useApp((s) => norm(s.source));
  const rescued = useImage((s) => s.tiles.rescued);
  const scanning = useScan((s) => norm(s.scanning));
  const surveying = useScan((s) => s.surveying.includes(key));
  const hashing = useVerify((s) => norm(s.hashing));
  const lastHash = useVerify((s) => s.results.find((r) => norm(r.source) === key)?.sha256);

  if (running === "image" && imageSrc === key) return { text: `Imaging · ${rescued} rescued`, busy: true };
  if (running === "scan" && scanning === key) return { text: "Scanning partitions", busy: true };
  if (running === "hash" && hashing === key) return { text: "Computing SHA-256", busy: true };
  if (surveying) return { text: "Reading partition table", busy: true };
  if (lastHash) return { text: `SHA-256 ${lastHash.slice(0, 10)}…`, busy: false };
  return { text: "", busy: false };
}
