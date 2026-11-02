// Pick a partition the way you'd point at it: the drive drawn to scale on top, one card per
// partition below, and a highlight that glides to whichever is chosen.
import { motion } from "motion/react";
import type { Partition } from "../lib/contract";
import { fmtBytes } from "../lib/format";
import { useApp } from "../store/app";
import { useScan } from "../store/scan";
import { PartitionMap, fsName, fsTone } from "./PartitionMap";
import { Busy, cx } from "./ui";

const SECTOR = 512;

export function PartitionPicker({
  id,
  parts,
  size,
  source,
  selected,
  onSelect,
  whole,
  unusable,
}: {
  /** Distinguishes pickers on the same page (their highlights must not fly between them). */
  id: string;
  parts: Partition[] | null;
  size: number | null;
  source: string;
  /** start_lba of the chosen partition; null = the whole source (when `whole` is offered) or nothing. */
  selected: number | null;
  onSelect: (lba: number | null) => void;
  /** Label for an extra "whole source" choice. */
  whole?: string;
  /** Why a partition can't be picked here, or null if it can. */
  unusable?: (p: Partition) => string | null;
}) {
  // `scanning` keeps the last source after the scan ends, so only `running` says one is under way.
  const scanning = useScan((s) => s.scanning);
  const running = useApp((s) => s.running);
  if (running === "scan" && scanning.trim().toLowerCase() === source.trim().toLowerCase()) return <Busy>Reading the partition table of {source.trim()}...</Busy>;
  if (!parts) return <p className="dim flush">{source.trim() ? "Partitions show here once the source is read." : "Choose a source above."}</p>;

  const sel = parts.findIndex((p) => p.start_lba === selected);
  return (
    <div className="ppick">
      {size ? <PartitionMap parts={parts} size={size} large selected={sel < 0 ? null : sel} onSelect={(i) => !unusable?.(parts[i]!) && onSelect(parts[i]!.start_lba)} /> : null}
      <div className="pcards" role="radiogroup" aria-label="Partition">
        {whole !== undefined && (
          <Card id={id} on={selected === null} index={0} onClick={() => onSelect(null)} tone="fs-whole">
            <b>{whole}</b>
            <span className="dim">{size ? fmtBytes(size) : "Every sector"}</span>
          </Card>
        )}
        {parts.length === 0 && <p className="dim flush">No partitions found. Carving the whole source still works.</p>}
        {parts.map((p, i) => {
          const why = unusable?.(p) ?? null;
          return (
            <Card key={p.start_lba} id={id} on={p.start_lba === selected} index={i + 1} disabled={!!why} onClick={() => onSelect(p.start_lba)} tone={fsTone(p.filesystem)}>
              <b>
                {fsName(p.filesystem)}
                {p.label && <span className="dim"> {p.label}</span>}
              </b>
              <span className="m">{p.size_sectors === null ? "size unknown" : fmtBytes(p.size_sectors * SECTOR)}</span>
              <span className="dim small">{why ?? `Starts at sector ${p.start_lba}`}</span>
            </Card>
          );
        })}
      </div>
    </div>
  );
}

function Card({
  id,
  on,
  index,
  disabled,
  tone,
  onClick,
  children,
}: {
  id: string;
  on: boolean;
  index: number;
  disabled?: boolean;
  tone: string;
  onClick: () => void;
  children: React.ReactNode;
}) {
  return (
    <motion.button
      type="button"
      role="radio"
      aria-checked={on}
      disabled={disabled}
      className={cx("pcard", tone, on && "on")}
      onClick={onClick}
      initial={{ opacity: 0, y: 8 }}
      animate={{ opacity: disabled ? 0.45 : 1, y: 0 }}
      transition={{ type: "spring", bounce: 0.2, visualDuration: 0.3, delay: index * 0.04 }}
      whileHover={disabled ? undefined : { y: -2 }}
      whileTap={disabled ? undefined : { scale: 0.98 }}
    >
      {on && <motion.span className="pcard-ring" layoutId={`pick-${id}`} transition={{ type: "spring", bounce: 0.25, visualDuration: 0.35 }} />}
      {children}
    </motion.button>
  );
}
