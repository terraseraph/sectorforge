import { useShallow } from "zustand/react/shallow";
import { fmtCount, percent } from "../lib/format";
import { goTo } from "../store/actions";
import { useApp } from "../store/app";
import { useImage } from "../store/image";
import { files, useRecover } from "../store/recover";
import { useScan } from "../store/scan";
import { useVerify } from "../store/verify";
import type { Tool } from "./tools";

export interface ToolStatus {
  /** One short line, or "" for nothing worth saying. */
  text: string;
  busy: boolean;
  /** 0..100 while busy and the total is known. */
  pct: number | null;
}

const IDLE_STATUS: ToolStatus = { text: "", busy: false, pct: null };

/** What a tool is doing right now, for its Home card and menu entry. */
export function useToolStatus(tool: Tool): ToolStatus {
  const running = useApp((s) => s.running);
  const devices = useApp((s) => s.devices.length);
  const devicesState = useApp((s) => s.devicesState);
  const image = useImage(useShallow((s) => ({ hasMap: s.hasMap, mapDone: s.mapDone, rescued: s.tiles.rescued, bad: s.tiles.bad, retryOnly: s.retryOnly })));
  const scan = useScan(useShallow((s) => ({ pct: s.pct, n: s.result?.partitions.length ?? null })));
  const verify = useVerify(useShallow((s) => ({ size: s.size, done: s.done, last: s.results[0]?.sha256 })));
  const listing = useRecover((s) => s.listing);
  useRecover((s) => s.filesVersion);

  switch (tool.id) {
    case "image":
    case "retry": {
      const mine = running === "image" && (tool.id === "retry") === image.retryOnly;
      if (mine) return { text: `Running · ${image.rescued} rescued`, busy: true, pct: parseFloat(image.rescued) || null };
      if (tool.id === "retry") return image.hasMap && image.bad !== "–" ? { ...IDLE_STATUS, text: `Map has ${image.bad} bad` } : IDLE_STATUS;
      if (image.mapDone) return { ...IDLE_STATUS, text: `Finished · ${image.rescued} rescued` };
      return image.hasMap ? { ...IDLE_STATUS, text: `Part done · ${image.rescued}, can resume` } : IDLE_STATUS;
    }
    case "drives":
      return devicesState === "loading" ? { text: "Looking for drives", busy: true, pct: null } : { ...IDLE_STATUS, text: devices ? `${devices} drives attached` : "" };
    case "partitions":
      if (running === "scan") return { text: "Scanning", busy: true, pct: scan.pct || null };
      return scan.n === null ? IDLE_STATUS : { ...IDLE_STATUS, text: `${scan.n} partitions found` };
    case "mapview":
      return image.hasMap ? { ...IDLE_STATUS, text: `Current map · ${image.rescued} rescued` } : IDLE_STATUS;
    case "verify":
      if (running === "hash") return { text: "Hashing", busy: true, pct: verify.size ? percent(verify.done, verify.size) : null };
      return verify.last ? { ...IDLE_STATUS, text: `Last hash ${verify.last.slice(0, 12)}…` } : IDLE_STATUS;
    case "recover":
      if (running === "files" || running === "extract" || running === "carve") return { text: `Running ${running}`, busy: true, pct: null };
      return listing === "done" ? { ...IDLE_STATUS, text: `${fmtCount(files.count)} files listed` } : IDLE_STATUS;
  }
  return IDLE_STATUS;
}

export function openTool(tool: Tool) {
  if (tool.id === "image" || tool.id === "retry") useImage.setState({ retryOnly: tool.id === "retry" });
  goTo(tool.screen);
}
