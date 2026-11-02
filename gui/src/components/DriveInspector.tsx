// Slide-in panel for one drive (or scanned image): everything known about it, its partitions
// drawn to scale, and every action that can start from it.
import { AlertTriangle, FileSearch, Fingerprint, HardDriveDownload, Layers, type LucideIcon, RotateCcw, ScanSearch, X } from "lucide-react";
import { AnimatePresence, motion } from "motion/react";
import { useEffect, useState } from "react";
import { createPortal } from "react-dom";
import type { Device } from "../lib/contract";
import { busLabel, deviceTitle } from "../lib/devices";
import { fmtBytes } from "../lib/format";
import { CONFIDENCE } from "../screens/PartitionsScreen";
import { goTo, startScan } from "../store/actions";
import { useApp } from "../store/app";
import { useImage } from "../store/image";
import { useRecover } from "../store/recover";
import { useScan } from "../store/scan";
import { useVerify } from "../store/verify";
import { fsName, PartitionLegend, PartitionMap } from "./PartitionMap";
import { Badge, Notice } from "./ui";

export interface InspectItem {
  path: string;
  title: string;
  size: number | null;
  icon: LucideIcon;
  device: Device | null;
}

const norm = (p: string) => p.trim().toLowerCase();

export function DriveInspector({ item, onClose }: { item: InspectItem | null; onClose: () => void }) {
  useEffect(() => {
    if (!item) return;
    const esc = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    document.addEventListener("keydown", esc);
    return () => document.removeEventListener("keydown", esc);
  }, [item, onClose]);

  const host = document.querySelector(".content");
  if (!host) return null;
  return createPortal(
    <AnimatePresence>
      {item && (
        <>
          <motion.div className="inspector-backdrop" initial={{ opacity: 0 }} animate={{ opacity: 1 }} exit={{ opacity: 0 }} onClick={onClose} />
          <motion.aside
            key={item.path}
            className="inspector"
            aria-label={`Drive: ${item.title}`}
            initial={{ x: "100%" }}
            animate={{ x: 0 }}
            exit={{ x: "100%" }}
            transition={{ type: "spring", bounce: 0, visualDuration: 0.3 }}
          >
            <Body item={item} onClose={onClose} />
          </motion.aside>
        </>
      )}
    </AnimatePresence>,
    host,
  );
}

function Body({ item, onClose }: { item: InspectItem; onClose: () => void }) {
  const key = norm(item.path);
  const scan = useScan((s) => s.bySource[key]);
  const readError = useScan((s) => s.errors[key]);
  const surveying = useScan((s) => s.surveying.includes(key));
  const running = useApp((s) => s.running);
  const hash = useVerify((s) => s.results.find((r) => norm(r.source) === key));
  const [sel, setSel] = useState<number | null>(null);
  const d = item.device;
  const Icon = item.icon;
  const part = sel !== null ? scan?.partitions[sel] : undefined;

  const act = (fn: () => void) => () => {
    onClose();
    fn();
  };
  const actions: { icon: LucideIcon; label: string; hint: string; run: () => void; disabled?: boolean; hidden?: boolean; primary?: boolean; stay?: boolean }[] = [
    {
      icon: HardDriveDownload,
      label: "Image this drive",
      hint: d?.system ? "System drive: you'll be asked to confirm" : "Copy it to a file; damaged areas are left for last",
      hidden: !d,
      primary: !!readError,
      run: () => {
        useApp.setState({ source: item.path });
        useImage.setState({ retryOnly: false });
        goTo("image");
      },
    },
    {
      icon: RotateCcw,
      label: "Quick rescan",
      hint: "Read the partition table again (a few KB)",
      disabled: !!running,
      stay: true,
      run: () => {
        useApp.setState({ source: item.path });
        void startScan(item.path, false);
      },
    },
    {
      icon: ScanSearch,
      label: "Deep scan",
      hint: "Search every sector for lost partitions (slow)",
      run: () => {
        useApp.setState({ source: item.path });
        useScan.setState({ deep: true });
        goTo("partitions");
      },
    },
    {
      icon: FileSearch,
      label: "Recover files",
      hint: d ? "Straight from the drive; image it first if it's failing" : "Browse or carve this image",
      run: () => {
        useRecover.setState({ source: item.path, sourceEdited: true });
        goTo("recover");
      },
    },
    {
      icon: Fingerprint,
      label: "Verify (SHA-256)",
      hint: "Reads every byte once",
      run: () => {
        useVerify.setState({ source: item.path });
        goTo("verify");
      },
    },
    {
      icon: Layers,
      label: "Partitions screen",
      hint: "Full table with offsets",
      run: () => {
        useApp.setState({ source: item.path });
        goTo("partitions");
      },
    },
  ];

  const facts: [string, React.ReactNode][] = d
    ? [
        ["Size", d.size === null ? "unknown" : `${fmtBytes(d.size)} (${d.size.toLocaleString("en-US")} bytes)`],
        ["Connection", busLabel(d.bus)],
        ["Sector size", d.sector_size ? `${d.sector_size} bytes` : "unknown"],
        ["Serial number", d.serial?.trim() || <span className="dim">not reported</span>],
        ["Path", <span className="mono">{d.path}</span>],
        ["Table", scan ? (scan.kind === "none" ? "No partition table" : scan.kind.toUpperCase()) : <span className="dim">not read yet</span>],
      ]
    : [
        ["Size", fmtBytes(item.size)],
        ["File", <span className="mono">{item.path}</span>],
        ["Table", scan ? (scan.kind === "none" ? "No partition table" : scan.kind.toUpperCase()) : "not read yet"],
      ];

  return (
    <>
      <header className="insp-head">
        <span className="insp-icon">
          <Icon size={28} strokeWidth={1.6} aria-hidden="true" />
        </span>
        <span className="insp-title">
          <b>{item.title}</b>
          <span>
            {d?.system && <Badge tone="red">System drive</Badge>} {d?.removable && <Badge tone="dim">Removable</Badge>}{" "}
            {d && <span className="dim small">{deviceTitle(d) === item.title ? "" : deviceTitle(d)}</span>}
          </span>
        </span>
        <button type="button" className="insp-close" aria-label="Close" onClick={onClose}>
          <X size={18} />
        </button>
      </header>

      <div className="insp-scroll">
        {readError && (
          <Notice tone="danger">
            <p>
              <AlertTriangle size={14} aria-hidden="true" /> <b>This drive returned a read error</b> on its last scan: <span className="mono small">{readError}</span>
            </p>
            <p>
              That often means the drive is failing. Every read can make it worse, so if anything on it matters, image it before doing
              anything else, and check its health.
            </p>
          </Notice>
        )}

        <section>
          <h3>Partitions</h3>
          {surveying || running === "scan" ? (
            <p className="dim small">Reading the partition table...</p>
          ) : !scan ? (
            <p className="dim small">
              {readError ? "The partition table could not be read." : "Not read yet. Quick rescan below reads just the partition table."}
            </p>
          ) : null}
          <PartitionMap parts={scan?.partitions ?? null} size={item.size} large selected={sel} onSelect={(i) => setSel(i === sel ? null : i)} />
          {scan && <PartitionLegend />}
          {scan && scan.partitions.length > 0 && (
            <table className="ptable">
              <thead>
                <tr>
                  <th>Filesystem</th>
                  <th>Label</th>
                  <th>Start</th>
                  <th>Size</th>
                  <th>Confidence</th>
                </tr>
              </thead>
              <tbody>
                {scan.partitions.map((p, i) => {
                  const c = CONFIDENCE[p.confidence];
                  return (
                    <tr key={p.start_lba} className={i === sel ? "sel" : undefined} onClick={() => setSel(i === sel ? null : i)}>
                      <td>{fsName(p.filesystem)}</td>
                      <td className="dim">{p.label ?? ""}</td>
                      <td className="m" title={`LBA ${p.start_lba}`}>
                        {fmtBytes(p.start_lba * 512)}
                      </td>
                      <td className="m">{p.size_sectors === null ? "?" : fmtBytes(p.size_sectors * 512)}</td>
                      <td>{c ? <Badge tone={c.tone}>{c.label}</Badge> : p.confidence}</td>
                    </tr>
                  );
                })}
              </tbody>
            </table>
          )}
          <AnimatePresence>
            {part && sel !== null && (
              <motion.div className="insp-part" initial={{ opacity: 0, height: 0 }} animate={{ opacity: 1, height: "auto" }} exit={{ opacity: 0, height: 0 }}>
                <p className="small">
                  <b>{fsName(part.filesystem)}</b> {part.label && <span className="dim">{part.label}</span>} at LBA <span className="mono">{part.start_lba}</span>
                  {part.filesystem === "ntfs" ? ". Its files can be listed, including deleted ones." : part.filesystem === "unknown" ? ". No filesystem SectorForge can read; carving still works." : ". File listing supports NTFS only; carving works on any partition."}
                </p>
                {part.filesystem === "ntfs" && (
                  <button
                    type="button"
                    className="btn small primary"
                    onClick={act(() => {
                      useRecover.setState({ source: item.path, sourceEdited: true, lba: part.start_lba, tab: "files" });
                      goTo("recover");
                    })}
                  >
                    Recover files from this partition
                  </button>
                )}
              </motion.div>
            )}
          </AnimatePresence>
        </section>

        <section>
          <h3>Details</h3>
          <table className="kv">
            <tbody>
              {facts.map(([k, v]) => (
                <tr key={k}>
                  <th>{k}</th>
                  <td>{v}</td>
                </tr>
              ))}
              {hash && (
                <tr>
                  <th>SHA-256</th>
                  <td className="mono small">{hash.sha256}</td>
                </tr>
              )}
            </tbody>
          </table>
        </section>

        <section>
          <h3>Do something with it</h3>
          <div className="insp-actions">
            {actions
              .filter((a) => !a.hidden)
              .map((a, i) => (
                <motion.button
                  key={a.label}
                  type="button"
                  className={a.primary ? "primary" : undefined}
                  disabled={a.disabled}
                  onClick={a.stay ? a.run : act(a.run)}
                  initial={{ opacity: 0, y: 6 }}
                  animate={{ opacity: 1, y: 0 }}
                  transition={{ delay: 0.1 + i * 0.03, duration: 0.18 }}
                >
                  <a.icon size={16} aria-hidden="true" />
                  <span>
                    <b>{a.label}</b>
                    <small>{a.disabled ? "Wait for the current job to finish" : a.hint}</small>
                  </span>
                </motion.button>
              ))}
          </div>
        </section>
      </div>
    </>
  );
}
