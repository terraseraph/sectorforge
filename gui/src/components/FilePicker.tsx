import { ArrowUp, File, Folder, FolderPlus, HardDrive } from "lucide-react";
import { AnimatePresence, motion } from "motion/react";
import { useEffect, useState } from "react";
import { type DirEntry, type Place, api, errorText } from "../lib/api";
import { fmtBytes } from "../lib/format";
import { crumbsOf, joinFs, parentOf, splitFs } from "../lib/paths";
import { showDialog } from "../store/dialog";
import { type PickRequest, usePicker } from "../store/picker";
import { Busy, Button, TextInput, cx } from "./ui";

export function FilePicker() {
  const req = usePicker((s) => s.req);
  return req && <Picker key={req.id} req={req} />;
}

const ACTION = { save: "Save here", folder: "Use this folder", open: "Open" } as const;

function Picker({ req }: { req: PickRequest }) {
  const start = req.initial ? splitFs(req.initial) : { dir: null, name: "" };
  const [dir, setDir] = useState<string | null>(start.dir ?? usePicker.getState().lastDir);
  const [name, setName] = useState(req.mode === "save" ? start.name : "");
  const [places, setPlaces] = useState<Place[] | null>(null);
  const [entries, setEntries] = useState<DirEntry[] | null>(null);
  const [error, setError] = useState("");
  const [reload, setReload] = useState(0);

  useEffect(() => {
    api.listPlaces().then(setPlaces, (e) => setError(errorText(e)));
  }, []);
  useEffect(() => {
    setEntries(null);
    setError("");
    if (dir !== null) api.listDir(dir).then(setEntries, (e) => setError(errorText(e)));
  }, [dir, reload]);

  const choose = () => {
    if (dir === null) return;
    const path = req.mode === "folder" ? dir : name.trim() ? joinFs(dir, name.trim()) : null;
    if (path === null) return;
    usePicker.setState({ lastDir: dir });
    req.resolve(path);
  };
  const newFolder = async () => {
    if (dir === null) return;
    const n = await showDialog({ title: "New folder", input: "", okLabel: "Create" });
    if (!n?.trim()) return;
    try {
      await api.createFolder(joinFs(dir, n.trim()));
      setDir(joinFs(dir, n.trim()));
    } catch (e) {
      setError(errorText(e));
    }
  };
  const place = places?.find((p) => dir?.toLowerCase().startsWith(p.path.toLowerCase()));
  const ready = dir !== null && (req.mode === "folder" || name.trim() !== "");

  return (
    <div className="dialog-backdrop" onMouseDown={(e) => e.target === e.currentTarget && req.resolve(null)}>
      <form
        className="dialog picker"
        role="dialog"
        aria-modal="true"
        aria-labelledby="pick-title"
        onSubmit={(e) => {
          e.preventDefault();
          choose();
        }}
        onKeyDown={(e) => {
          if (e.key !== "Escape") return;
          e.stopPropagation();
          req.resolve(null);
        }}
      >
        <h3 id="pick-title">{req.title}</h3>
        {req.need !== undefined && <p className="dialog-msg">The image will need {fmtBytes(req.need)} of free space.</p>}

        <div className="places" role="group" aria-label="Drives">
          {places === null ? (
            <Busy>Finding drives...</Busy>
          ) : (
            places.map((p) => {
              const short = req.need !== undefined && p.free !== null && p.free < req.need;
              return (
                <button
                  key={p.path}
                  type="button"
                  className={cx("place", place?.path === p.path && "on", short && "short")}
                  onClick={() => setDir(p.path)}
                  title={short ? "Not enough free space for the image" : undefined}
                >
                  <HardDrive size={16} aria-hidden="true" />
                  <span className="pl">
                    <b>{p.path.replace(/\\$/, "")}</b> {p.label}
                  </span>
                  {p.free !== null && p.total ? (
                    <>
                      <span className="pbar" aria-hidden="true">
                        <i style={{ width: `${100 - (p.free / p.total) * 100}%` }} />
                      </span>
                      <span className="pfree">{fmtBytes(p.free)} free</span>
                    </>
                  ) : null}
                </button>
              );
            })
          )}
        </div>

        <div className="pick-nav">
          <Button small disabled={dir === null} onClick={() => setDir(dir && parentOf(dir))} aria-label="Up one folder">
            <ArrowUp size={14} aria-hidden="true" />
          </Button>
          <nav className="crumbs" aria-label="Folder">
            {dir === null ? (
              <span className="dim">Pick a drive</span>
            ) : (
              crumbsOf(dir).map((c, i, all) => (
                <span key={c.path} className="crumb">
                  {i > 0 && <span className="sep">{dir.startsWith("/") ? "/" : "\\"}</span>}
                  {i === all.length - 1 ? (
                    <b>{c.label}</b>
                  ) : (
                    <Button variant="link" onClick={() => setDir(c.path)}>
                      {c.label}
                    </Button>
                  )}
                </span>
              ))
            )}
          </nav>
          {req.mode !== "open" && (
            <Button small disabled={dir === null} onClick={() => void newFolder()}>
              <FolderPlus size={14} aria-hidden="true" /> New folder
            </Button>
          )}
        </div>

        <div className="pick-list" role="listbox" aria-label="Folders and files">
          <AnimatePresence mode="wait" initial={false}>
            <motion.div key={`${dir}|${reload}`} initial={{ opacity: 0, x: 8 }} animate={{ opacity: 1, x: 0 }} exit={{ opacity: 0, x: -8 }} transition={{ duration: 0.12 }}>
              {error ? (
                <p className="dim pick-empty">
                  {error}{" "}
                  <Button variant="link" onClick={() => setReload((n) => n + 1)}>
                    Try again
                  </Button>
                </p>
              ) : dir === null ? (
                <p className="dim pick-empty">Choose a drive above.</p>
              ) : entries === null ? (
                <Busy>Reading the folder...</Busy>
              ) : entries.length === 0 ? (
                <p className="dim pick-empty">This folder is empty.</p>
              ) : (
                entries.map((e) => (
                  <button
                    key={e.name}
                    type="button"
                    role="option"
                    aria-selected={!e.dir && e.name === name}
                    className={cx("pick-row", !e.dir && req.mode === "folder" && "dim")}
                    disabled={!e.dir && req.mode === "folder"}
                    onClick={() => (e.dir ? setDir(joinFs(dir, e.name)) : setName(e.name))}
                    onDoubleClick={() => !e.dir && req.mode === "open" && req.resolve(joinFs(dir, e.name))}
                  >
                    {e.dir ? <Folder size={15} aria-hidden="true" /> : <File size={15} aria-hidden="true" />}
                    <span className="nm">{e.name}</span>
                    <span className="m dim">{e.dir ? "" : fmtBytes(e.size)}</span>
                  </button>
                ))
              )}
            </motion.div>
          </AnimatePresence>
        </div>

        {req.mode === "save" && (
          <div className="field">
            <label htmlFor="pick-name">File name</label>
            <TextInput id="pick-name" value={name} onChange={(e) => setName(e.target.value)} />
            <span />
          </div>
        )}
        {req.mode === "open" && name && (
          <p className="dim small">
            Selected: <span className="mono">{name}</span>
          </p>
        )}
        {place && req.need !== undefined && place.free !== null && place.free < req.need && (
          <p className="warn-text">
            {place.path} has {fmtBytes(place.free)} free, less than the {fmtBytes(req.need)} the image needs.
          </p>
        )}
        <div className="actions">
          <Button onClick={() => req.resolve(null)}>Cancel</Button>
          <Button variant="primary" type="submit" disabled={!ready}>
            {ACTION[req.mode]}
          </Button>
        </div>
      </form>
    </div>
  );
}
