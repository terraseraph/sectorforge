import { useEffect, useMemo, useState } from "react";
import { VirtualList } from "../../components/VirtualList";
import { Badge, Button, Check, type Tone, cx } from "../../components/ui";
import { type FileKind, KINDS, ORPHANS, REC_NAME, type SortKey, crumbs, extOf, noFilter } from "../../lib/filestore";
import { fmtBytes, fmtCount, fmtDate } from "../../lib/format";
import { clearSelection, setSelected } from "../../store/actions";
import { NO_FILTER, files, selection, useRecover } from "../../store/recover";

const ROW_H = 30;
const SEARCH_DEBOUNCE_MS = 200;

const RECOVERABLE: Record<(typeof REC_NAME)[number], { label: string; tone: Tone }> = {
  full: { label: "Fully recoverable", tone: "green" },
  partial: { label: "Partly recoverable", tone: "amber" },
  none: { label: "Not recoverable", tone: "red" },
};

const navigate = (folder: string) => useRecover.setState({ folder });
const clearFilters = () => useRecover.setState({ filter: NO_FILTER });

// Size and date read best biggest/newest first; name and type A to Z.
const FIRST_DESC: Record<SortKey, boolean> = { name: false, type: false, modified: true, size: true };
const sortOn = (key: SortKey) =>
  useRecover.setState((s) => ({ sort: { key, desc: s.sort?.key === key ? !s.sort.desc : FIRST_DESC[key] } }));

export function FileBrowser() {
  const filesVersion = useRecover((s) => s.filesVersion);
  const selVersion = useRecover((s) => s.selVersion);
  const folder = useRecover((s) => s.folder);
  const filter = useRecover((s) => s.filter);
  const listing = useRecover((s) => s.listing);
  const sort = useRecover((s) => s.sort);
  const flat = !noFilter(filter);

  // filesVersion stands in for the (mutable) store's contents.
  const view = useMemo(() => {
    const base = flat ? files.search(filter) : files.list(folder);
    return sort ? files.sortBy(base, sort) : base;
  }, [flat, filter, folder, sort, filesVersion]);
  const allShownSelected = useMemo(() => view.length > 0 && view.every((i) => selection.has(i)), [view, selVersion]);

  const empty = listing === "running" ? "Reading the file table..." : flat ? "Nothing matches these filters." : "This folder is empty.";

  return (
    <>
      <Crumbs flat={flat} folder={folder} />
      <Filters count={view.length} flat={flat} />
      <div className="frow fhead">
        <input
          type="checkbox"
          aria-label="Select everything shown"
          checked={allShownSelected}
          disabled={view.length === 0}
          onChange={(e) => setSelected(view, e.target.checked)}
        />
        <SortHead k="name">Name</SortHead>
        <SortHead k="type">Type</SortHead>
        <SortHead k="modified">Modified (UTC)</SortHead>
        <SortHead k="size">Size</SortHead>
        <span>State</span>
        <span>Recoverable</span>
      </div>
      <VirtualList
        count={view.length}
        rowHeight={ROW_H}
        label="Files and folders"
        empty={empty}
        resetKey={`${flat ? `search:${JSON.stringify(filter)}` : folder}|${JSON.stringify(sort)}`}
        renderRow={(k) => <FileRow index={view[k]!} flat={flat} />}
      />
      <SelectionSummary />
    </>
  );
}

function SortHead({ k, children }: { k: SortKey; children: string }) {
  const sort = useRecover((s) => s.sort);
  const on = sort?.key === k;
  return (
    <span>
      <button type="button" className={cx("sorth", on && "on")} title={`Sort by ${children.toLowerCase()}`} onClick={() => sortOn(k)}>
        {children}
        {on && <span aria-label={sort.desc ? ", descending" : ", ascending"}>{sort.desc ? " ▼" : " ▲"}</span>}
      </button>
    </span>
  );
}

function Crumbs({ flat, folder }: { flat: boolean; folder: string }) {
  useRecover((s) => s.filesVersion);
  if (flat) {
    return (
      <div className="crumbs">
        <span className="dim">Showing matches from the whole partition.</span>
        <Button variant="link" onClick={clearFilters}>
          Back to folders
        </Button>
      </div>
    );
  }
  const path = crumbs(folder);
  const orphans = files.orphanCount;
  return (
    <nav className="crumbs" aria-label="Folder">
      {path.map((c, i) => (
        <span key={c.dir} className="crumb">
          {i > 0 && <span className="sep">/</span>}
          {i === path.length - 1 && folder !== ORPHANS ? (
            <b>{c.label}</b>
          ) : (
            <Button variant="link" onClick={() => navigate(c.dir)}>
              {c.label}
            </Button>
          )}
        </span>
      ))}
      {orphans > 0 && (
        <>
          <span className="sep">|</span>
          {folder === ORPHANS ? (
            <b>Orphaned files ({fmtCount(orphans)})</b>
          ) : (
            <Button variant="link" onClick={() => navigate(ORPHANS)}>
              Orphaned files ({fmtCount(orphans)})
            </Button>
          )}
        </>
      )}
    </nav>
  );
}

function Filters({ count, flat }: { count: number; flat: boolean }) {
  const filter = useRecover((s) => s.filter);
  const [text, setText] = useState(filter.text);
  useRecover((s) => s.filesVersion);

  // Clearing filters elsewhere ("Back to folders") must clear the box too.
  useEffect(() => setText(filter.text), [filter.text]);
  useEffect(() => {
    if (text === filter.text) return;
    const t = setTimeout(() => useRecover.setState((s) => ({ filter: { ...s.filter, text } })), SEARCH_DEBOUNCE_MS);
    return () => clearTimeout(t);
  }, [text, filter.text]);

  const setFlag = (key: "deletedOnly" | "recoverableOnly") => (on: boolean) =>
    useRecover.setState((s) => ({ filter: { ...s.filter, [key]: on } }));
  const setKind = (kind: FileKind) => (on: boolean) =>
    useRecover.setState((s) => ({ filter: { ...s.filter, kinds: on ? [...s.filter.kinds, kind] : s.filter.kinds.filter((k) => k !== kind) } }));
  const total = fmtCount(files.count);

  return (
    <div className="filters">
      <input type="text" spellCheck={false} placeholder="Search by name" aria-label="Search by name" value={text} onChange={(e) => setText(e.target.value)} />
      <Check checked={filter.deletedOnly} onChange={setFlag("deletedOnly")}>
        Deleted only
      </Check>
      <Check checked={filter.recoverableOnly} onChange={setFlag("recoverableOnly")}>
        Recoverable only
      </Check>
      <span className="kinds" role="group" aria-label="File types">
        {KINDS.map((k) => (
          <Check key={k.id} checked={filter.kinds.includes(k.id)} onChange={setKind(k.id)}>
            {k.label}
          </Check>
        ))}
      </span>
      <span className="dim count">
        {flat
          ? `${fmtCount(count)} matches (of ${total} entries)`
          : `${fmtCount(count)} items here (${total} entries in all, ${fmtCount(files.deletedCount)} deleted)`}
      </span>
    </div>
  );
}

function FileRow({ index: i, flat }: { index: number; flat: boolean }) {
  const selected = selection.has(i);
  const dir = files.isDir(i);
  const name = files.names[i] ?? "";
  const rec = RECOVERABLE[REC_NAME[files.recov[i] ?? 2]];
  const folder = files.folderOf(i);

  return (
    <div className={cx("frow", selected && "sel")}>
      <input type="checkbox" aria-label={`Select ${name}`} checked={selected} onChange={(e) => setSelected([i], e.target.checked)} />
      <span className="nm">
        {dir ? (
          <Button
            variant="link"
            onClick={() => {
              if (flat) clearFilters();
              navigate(files.pathOf(i));
            }}
          >
            {name}
          </Button>
        ) : (
          name
        )}
        {flat && (
          <Button
            variant="link"
            className="in-folder"
            onClick={() => {
              clearFilters();
              navigate(folder);
            }}
          >
            in {folder === ORPHANS ? "Orphaned files" : folder}
          </Button>
        )}
      </span>
      <span className="m dim">{dir ? "" : extOf(name).toUpperCase()}</span>
      <span className="m dim">{fmtDate(files.modified[i])}</span>
      <span className="m">{dir ? "Folder" : fmtBytes(files.sizes[i] ?? 0)}</span>
      <span>{files.isDeleted(i) && <Badge tone="amber">Deleted</Badge>}</span>
      <span>
        <Badge tone={rec.tone}>{rec.label}</Badge>
      </span>
    </div>
  );
}

function SelectionSummary() {
  const selVersion = useRecover((s) => s.selVersion);
  const summary = useMemo(() => {
    let bytes = 0;
    let dirs = 0;
    for (const i of selection) {
      if (files.isDir(i)) dirs++;
      else bytes += files.sizes[i] ?? 0;
    }
    if (!selection.size) return "Nothing selected. Tick files or folders to extract.";
    return `${fmtCount(selection.size)} selected${dirs ? ` (${fmtCount(dirs)} folders, plus everything inside)` : `, ${fmtBytes(bytes)}`}`;
  }, [selVersion]);

  return (
    <div className="actions between">
      <span className="dim">{summary}</span>
      <Button small disabled={selection.size === 0} onClick={clearSelection}>
        Clear selection
      </Button>
    </div>
  );
}
