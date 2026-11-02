import { isDevicePath } from "../lib/format";
import { goTo } from "../store/actions";
import { type Screen, useApp } from "../store/app";
import { useRecoverSource } from "../store/hooks";

const SCREENS: { id: Screen; label: string }[] = [
  { id: "image", label: "Image" },
  { id: "partitions", label: "Partitions" },
  { id: "recover", label: "Recover" },
];

export function Nav() {
  const screen = useApp((s) => s.screen);
  return (
    <nav aria-label="Screens">
      <h1>SectorForge</h1>
      {SCREENS.map((s) => (
        <button key={s.id} type="button" aria-current={screen === s.id ? "page" : undefined} onClick={() => goTo(s.id)}>
          {s.label}
        </button>
      ))}
    </nav>
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
      <span>{activity}</span>
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
