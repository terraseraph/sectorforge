import { AnimatePresence, motion } from "motion/react";
import { useEffect, useLayoutEffect, useRef } from "react";
import { clearConsole, type LineKind, toggleConsole, useConsole } from "../store/console";
import { Button } from "./ui";

const KIND_LABEL: Record<LineKind, string> = { cmd: "$", out: "out", err: "err", files: "rows", exit: "exit" };
const time = (d: Date) => d.toLocaleTimeString([], { hour12: false });

/** Everything sf-cli printed this session: commands, JSON events, stderr, exits. Ctrl+` toggles it. */
export function Console() {
  const open = useConsole((s) => s.open);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.ctrlKey && e.key === "`") {
        e.preventDefault();
        toggleConsole();
      } else if (e.key === "Escape" && useConsole.getState().open) toggleConsole(false);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  return (
    <AnimatePresence>
      {open && (
        <motion.section
          className="console"
          aria-label="sf-cli console"
          initial={{ y: "100%" }}
          animate={{ y: 0 }}
          exit={{ y: "100%" }}
          transition={{ type: "spring", bounce: 0, visualDuration: 0.3 }}
        >
          <ConsoleBody />
        </motion.section>
      )}
    </AnimatePresence>
  );
}

function ConsoleBody() {
  const lines = useConsole((s) => s.lines);
  const box = useRef<HTMLDivElement>(null);
  const stick = useRef(true);

  // Follow new output unless the user has scrolled up to read something.
  useLayoutEffect(() => {
    const el = box.current;
    if (el && stick.current) el.scrollTop = el.scrollHeight;
  }, [lines]);

  const copy = () => void navigator.clipboard.writeText(lines.map((l) => `${time(l.at)} [${l.job}] ${KIND_LABEL[l.kind]} ${l.text}`).join("\n"));

  return (
    <>
      <div className="console-head">
        <b>Console</b>
        <span className="dim small">The exact sf-cli commands SectorForge ran, and everything they printed. Ctrl+` or Esc to close.</span>
        <span className="console-actions">
          <Button small onClick={copy} disabled={!lines.length}>
            Copy all
          </Button>
          <Button small onClick={clearConsole} disabled={!lines.length}>
            Clear
          </Button>
          <Button small onClick={() => toggleConsole(false)}>
            Close
          </Button>
        </span>
      </div>
      <div
        ref={box}
        className="console-lines"
        role="log"
        aria-live="off"
        onScroll={(e) => {
          const el = e.currentTarget;
          stick.current = el.scrollHeight - el.scrollTop - el.clientHeight < 24;
        }}
      >
        {lines.length === 0 ? (
          <p className="dim">Nothing yet. Start any job and its command and output appear here as they happen.</p>
        ) : (
          lines.map((l) => (
            <div key={l.id} className={`cl cl-${l.kind}`}>
              <span className="cl-t">{time(l.at)}</span>
              <span className="cl-j">{l.job}</span>
              <span className="cl-k">{KIND_LABEL[l.kind]}</span>
              <span className="cl-x">{l.text}</span>
            </div>
          ))
        )}
      </div>
    </>
  );
}

/** Status-bar button that opens the console; counts lines that arrived while it was closed. */
export function ConsoleToggle() {
  const open = useConsole((s) => s.open);
  const unseen = useConsole((s) => s.unseen);
  return (
    <button type="button" className="console-toggle" aria-expanded={open} onClick={() => toggleConsole()} title="Show sf-cli output (Ctrl+`)">
      Console
      <AnimatePresence>
        {unseen > 0 && (
          <motion.span key="n" className="console-count" initial={{ scale: 0 }} animate={{ scale: 1 }} exit={{ scale: 0 }}>
            {unseen > 999 ? "999+" : unseen}
          </motion.span>
        )}
      </AnimatePresence>
    </button>
  );
}
