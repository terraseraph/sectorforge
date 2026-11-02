import { create } from "zustand";

export type LineKind = "cmd" | "out" | "err" | "files" | "exit";
export interface ConsoleLine {
  id: number;
  at: Date;
  job: string;
  kind: LineKind;
  text: string;
}

/** Older lines are dropped past this; the console is for watching, not an archive. */
const KEEP = 1500;
let nextId = 0;

export const useConsole = create<{ open: boolean; lines: ConsoleLine[]; unseen: number }>(() => ({ open: false, lines: [], unseen: 0 }));

export function log(job: string, kind: LineKind, text: string) {
  const line = { id: nextId++, at: new Date(), job, kind, text };
  useConsole.setState((s) => ({
    lines: s.lines.length >= KEEP ? [...s.lines.slice(s.lines.length - KEEP + 1), line] : [...s.lines, line],
    unseen: s.open ? 0 : s.unseen + 1,
  }));
}

export const toggleConsole = (open = !useConsole.getState().open) => useConsole.setState({ open, unseen: 0 });
export const clearConsole = () => useConsole.setState({ lines: [], unseen: 0 });
