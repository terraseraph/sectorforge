import type { Job } from "../lib/contract";
import { isDevicePath } from "../lib/format";
import { goTo } from "../store/actions";
import { House } from "lucide-react";
import { motion } from "motion/react";
import { useApp } from "../store/app";
import { useImage } from "../store/image";
import { ConsoleToggle } from "./Console";
import { StatusChip } from "./StatusChip";
import { GROUP_TONE, TOOLS, type Tool } from "./tools";
import { openTool, useToolStatus } from "./toolStatus";
import { useRecoverSource } from "../store/hooks";
import { Busy, cx, Spinner } from "./ui";

const HOME: Tool = { ...TOOLS[0]!, id: "home", screen: "home", label: "Home", icon: House, sub: "Drives and tools", job: undefined };
const GROUPS = [...new Set(TOOLS.map((t) => t.group))];

export function Nav() {
  return (
    <nav aria-label="Screens">
      <h1>SectorForge</h1>
      <NavItem tool={HOME} />
      {GROUPS.map((g) => (
        <div key={g} className="nav-group">
          <h2 className={GROUP_TONE[g]}>{g}</h2>
          {TOOLS.filter((t) => t.group === g).map((t) => (
            <NavItem key={t.id} tool={t} />
          ))}
        </div>
      ))}
    </nav>
  );
}

function NavItem({ tool }: { tool: Tool }) {
  const screen = useApp((s) => s.screen);
  const retryOnly = useImage((s) => s.retryOnly);
  const status = useToolStatus(tool);
  // Image and Retry share a screen; the retry-only switch tells them apart.
  const current = screen === tool.screen && (tool.screen !== "image" || (tool.id === "retry") === retryOnly);
  return (
    <button type="button" aria-current={current ? "page" : undefined} onClick={() => (tool.id === "home" ? goTo("home") : openTool(tool))}>
      {current && <motion.span layoutId="nav-pill" className="nav-pill" transition={{ type: "spring", bounce: 0, visualDuration: 0.25 }} />}
      <span className="nav-label">
        <span className={cx("nav-icon", tool.id === "home" ? "tone-dim" : GROUP_TONE[tool.group])}>
          <tool.icon size={16} strokeWidth={2} aria-hidden="true" />
        </span>
        {tool.label}
        <StatusChip status={{ ...status, text: "" }} compact />
      </span>
      <small>{tool.sub}</small>
    </button>
  );
}

/** Shown when sf-cli is missing, too old, or speaks a different contract. */
export function CliBanner() {
  const cli = useApp((s) => s.cli);
  if (!cli?.problem) return null;
  return (
    <div className="banner" role="alert">
      <b>sf-cli problem.</b> {cli.problem}
      {cli.searched.length > 0 && (
        <details>
          <summary>Where SectorForge looked</summary>
          <ul>
            {cli.searched.map((p) => (
              <li key={p} className="mono">
                {p}
              </li>
            ))}
          </ul>
        </details>
      )}
    </div>
  );
}

const MODE_TEXT = {
  device: {
    name: "Direct hardware access",
    why: "SectorForge reads the physical drive itself. It needs Administrator rights and only ever reads from the drive. Reading a failing drive stresses it, so keep the number of passes low.",
  },
  file: {
    name: "Image file",
    why: "SectorForge is working on a file, not the drive, so the original hardware is not touched and the file can be re-read as often as needed.",
  },
} as const;

export function StatusBar() {
  const activity = useApp((s) => s.activity);
  const running = useApp((s) => s.running);
  const jobMode = useApp((s) => s.jobMode);
  const screen = useApp((s) => s.screen);
  const appSource = useApp((s) => s.source);
  const recoverSrc = useRecoverSource();

  const source = screen === "recover" ? recoverSrc : appSource.trim();
  const mode = running && jobMode ? jobMode : source ? (isDevicePath(source) ? "device" : "file") : "";
  return (
    <footer className="status" role="status" aria-live="polite">
      <span className="activity">
        <ConsoleToggle />
        {running && <Spinner />}
        {activity}
      </span>
      <span className="mode">
        {mode ? (
          <>
            Mode: <b>{MODE_TEXT[mode].name}</b>. {MODE_TEXT[mode].why}
          </>
        ) : (
          "Mode: choose a source to see how SectorForge will access it."
        )}
      </span>
    </footer>
  );
}

export function ErrorAlert() {
  const error = useApp((s) => s.error);
  if (!error) return null;
  return (
    <div className="alert" role="alert">
      {error}
    </div>
  );
}

/** While `job` runs: a spinner and the CLI's latest word on what it is doing. */
export function JobActivity({ job }: { job: Job }) {
  const running = useApp((s) => s.running);
  const activity = useApp((s) => s.activity);
  return running === job ? <Busy>{activity}</Busy> : null;
}
