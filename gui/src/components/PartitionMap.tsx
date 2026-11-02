// A drive drawn to scale, like Windows Disk Management: one segment per partition, gaps as
// unallocated space. Tiny partitions get a minimum width so they never disappear.
import { motion } from "motion/react";
import type { Partition } from "../lib/contract";
import { fmtBytes } from "../lib/format";
import { cx } from "./ui";

const SECTOR = 512;

export type Segment = { kind: "part"; p: Partition; index: number; start: number; bytes: number } | { kind: "free"; start: number; bytes: number };

/** Partitions in disk order with the unallocated gaps between them. Gaps under 0.5% (alignment slack) are left out. */
export function segments(parts: Partition[], size: number): Segment[] {
  const sorted = parts.map((p, index) => ({ p, index })).sort((a, b) => a.p.start_lba - b.p.start_lba);
  const out: Segment[] = [];
  const minGap = size * 0.005;
  let cursor = 0;
  sorted.forEach(({ p, index }, i) => {
    const start = p.start_lba * SECTOR;
    if (start - cursor > minGap) out.push({ kind: "free", start: cursor, bytes: start - cursor });
    // Unknown size: assume it runs to the next partition or the end of the drive.
    const next = sorted[i + 1]?.p.start_lba;
    const bytes = p.size_sectors !== null ? p.size_sectors * SECTOR : (next !== undefined ? next * SECTOR : size) - start;
    out.push({ kind: "part", p, index, start, bytes: Math.max(0, bytes) });
    cursor = Math.max(cursor, start + bytes);
  });
  if (size - cursor > minGap) out.push({ kind: "free", start: cursor, bytes: size - cursor });
  return out;
}

const FS_TONE: Record<string, string> = { ntfs: "fs-ntfs", fat12: "fs-fat", fat16: "fs-fat", fat32: "fs-fat", exfat: "fs-fat", ext: "fs-ext" };
export const fsTone = (fs: string) => FS_TONE[fs] ?? "fs-other";
export const fsName = (fs: string) => (fs === "unknown" ? "Unknown" : fs.toUpperCase());

export function PartitionMap({
  parts,
  size,
  large,
  selected,
  onSelect,
}: {
  parts: Partition[] | null;
  size: number | null;
  large?: boolean;
  selected?: number | null;
  onSelect?: (index: number) => void;
}) {
  if (!parts || !size) return <span className={cx("pmap empty", large && "large")} aria-hidden="true" />;
  const segs = segments(parts, size);
  return (
    <span className={cx("pmap", large && "large")} role="img" aria-label={`${parts.length} partitions`}>
      {segs.map((s, i) => {
        const label = s.kind === "part" ? `${fsName(s.p.filesystem)}${large && s.p.label ? ` · ${s.p.label}` : ""}` : "Unallocated";
        const title = `${s.kind === "part" ? `${fsName(s.p.filesystem)}${s.p.label ? ` (${s.p.label})` : ""}` : "Unallocated"}, ${fmtBytes(s.bytes)}`;
        const Tag = onSelect && s.kind === "part" ? motion.button : motion.span;
        return (
          <Tag
            key={s.start}
            type={onSelect && s.kind === "part" ? "button" : undefined}
            className={cx("seg", s.kind === "free" ? "free" : fsTone(s.p.filesystem), s.kind === "part" && s.p.confidence === "low_confidence" && "doubt", s.kind === "part" && selected === s.index && "sel")}
            style={{ flexGrow: s.bytes, minWidth: large ? 34 : 8 }}
            title={title}
            initial={{ opacity: 0, scaleY: 0.4 }}
            animate={{ opacity: 1, scaleY: 1 }}
            transition={{ type: "spring", bounce: 0, visualDuration: 0.35, delay: i * 0.05 }}
            onClick={onSelect && s.kind === "part" ? () => onSelect(s.index) : undefined}
          >
            <span className="seg-text">
              {label}
              {large && <small>{fmtBytes(s.bytes)}</small>}
            </span>
          </Tag>
        );
      })}
    </span>
  );
}

export function PartitionLegend() {
  return (
    <div className="legend">
      <span className="fs-ntfs">NTFS</span>
      <span className="fs-fat">FAT / exFAT</span>
      <span className="fs-ext">ext</span>
      <span className="fs-other">Other / unrecognised</span>
      <span className="free">Unallocated</span>
    </div>
  );
}
