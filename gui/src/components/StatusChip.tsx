import { AnimatePresence, motion } from "motion/react";
import type { ToolStatus } from "./toolStatus";
import { cx } from "./ui";

/** A tool's live state: a pulsing dot and mini progress bar while its job runs, a quiet note otherwise. */
export function StatusChip({ status, compact }: { status: ToolStatus; compact?: boolean }) {
  return (
    <AnimatePresence initial={false} mode="wait">
      {(status.busy || status.text) && (
        <motion.span
          // Keyed on busy, not text: the text changes several times a second while running.
          key={status.busy ? "busy" : "idle"}
          className={cx("chip", status.busy && "live", compact && "compact")}
          initial={{ opacity: 0, scale: 0.9 }}
          animate={{ opacity: 1, scale: 1 }}
          exit={{ opacity: 0, scale: 0.9 }}
          transition={{ duration: 0.18 }}
        >
          {status.busy && (
            <motion.i
              className="dot"
              aria-hidden="true"
              animate={{ opacity: [1, 0.25, 1] }}
              transition={{ duration: 1.2, repeat: Infinity, ease: "easeInOut" }}
            />
          )}
          {!compact && <span className="chip-text">{status.text}</span>}
          {status.busy && !compact && status.pct !== null && (
            <span className="chip-bar" aria-hidden="true">
              <span style={{ width: `${Math.min(100, Math.max(0, status.pct))}%` }} />
            </span>
          )}
        </motion.span>
      )}
    </AnimatePresence>
  );
}
