import { useEffect } from "react";
import { DrivePicker } from "../../components/DrivePicker";
import { ErrorAlert } from "../../components/Shell";
import { Button, Field, Hint, Notice, Panel, TextInput } from "../../components/ui";
import { dialogs } from "../../lib/api";
import { isDevicePath } from "../../lib/format";
import { recoverMap, startScan } from "../../store/actions";
import { useApp } from "../../store/app";
import { useRecoverSourceInput } from "../../store/hooks";
import { useImage } from "../../store/image";
import { useRecover } from "../../store/recover";
import { partitionsFor, useScan } from "../../store/scan";
import { CarveTab } from "./CarveTab";
import { FilesTab } from "./FilesTab";

const TABS = [
  { id: "files", label: "Browse files" },
  { id: "carve", label: "Carve by file type" },
] as const;

/** Reads the partition table once a source is settled on, unless it is already known, a job is running, or it failed before (Rescan retries). */
function autoScan(src: string) {
  const s = src.trim();
  const scan = useScan.getState();
  if (!s || partitionsFor(scan.result, s) || scan.errors[s.toLowerCase()] || useApp.getState().running) return;
  void startScan(s, false);
}

export function RecoverScreen() {
  const tab = useRecover((s) => s.tab);
  const input = useRecoverSourceInput();
  const source = input.trim();
  const setSource = (v: string) => useRecover.setState({ source: v, sourceEdited: true });
  const pickSource = (v: string) => {
    setSource(v);
    autoScan(v);
  };

  // Arriving with a source chosen elsewhere (e.g. the drive inspector): read it straight away.
  useEffect(() => {
    if (useRecover.getState().sourceEdited) autoScan(source);
  }, []);

  return (
    <>
      <h2>Recover files</h2>
      <p className="lead">
        Recover from an image or straight from a live drive, by filesystem (NTFS, FAT, exFAT, ext) or by carving sectors. Nothing here ever writes to the source.
      </p>
      <ErrorAlert />
      <div className="cols">
        <div>
          <DrivePicker selected={input} onPick={pickSource} />
        </div>
        <div>
          <Panel label="Source">
            <Field
              id="rsrc"
              label="Source"
              action={
                <Button
                  onClick={async () => {
                    const v = await dialogs.openFile("Choose an image file");
                    if (v !== null) pickSource(v);
                  }}
                >
                  Browse
                </Button>
              }
            >
              <TextInput id="rsrc" value={input} placeholder="\\.\PhysicalDrive1, /dev/sdb, or an image file" onChange={(e) => setSource(e.target.value)}
                onBlur={() => autoScan(input)}
                onKeyDown={(e) => e.key === "Enter" && autoScan(input)}
              />
            </Field>
            <SourceNotes source={source} />
          </Panel>
        </div>
      </div>

      <div className="tabs" role="tablist" aria-label="Recovery method">
        {TABS.map((t) => (
          <button
            key={t.id}
            type="button"
            role="tab"
            id={`tab-${t.id}`}
            aria-selected={tab === t.id}
            aria-controls="tabpanel"
            tabIndex={tab === t.id ? 0 : -1}
            onClick={() => useRecover.setState({ tab: t.id })}
          >
            {t.label}
          </button>
        ))}
      </div>
      <div id="tabpanel" role="tabpanel" aria-labelledby={`tab-${tab}`}>
        {tab === "files" ? <FilesTab source={source} /> : <CarveTab source={source} />}
      </div>
    </>
  );
}

function SourceNotes({ source }: { source: string }) {
  // Re-render when the map becomes known; recoverMap reads it.
  useImage((s) => s.hasMap);
  const map = recoverMap(source);
  return (
    <>
      <Hint flush>
        {map
          ? "Using the map from the Image screen, so files that overlap bad sectors are shown as partial instead of silently containing blank data."
          : "No map in use. It applies when the source is the image made on the Image screen; without it, files over damaged areas can contain blank data without a warning."}
      </Hint>
      {isDevicePath(source) && (
        <Notice tone="warn">
          <p>
            <b>Reading the live drive.</b> Save recovered files to a different drive, and stop writing to this one: new writes can overwrite
            deleted files. If the drive is failing, image it first on the Image screen and recover from the image instead.
          </p>
        </Notice>
      )}
    </>
  );
}
