import { useMemo } from "react";
import { cellsFromBlocks, type Status } from "../lib/map";
import { type ImageState, useImage } from "../store/image";
import { Panel } from "./ui";

export const CELLS = 1000;
const EMPTY: Status[] = new Array<Status>(CELLS).fill("non_tried");

/** Each cell is an equal slice of the drive, coloured by the worst status found in it. */
export function HealthMap({ snapshot: given }: { snapshot?: ImageState["snapshot"] }) {
  const stored = useImage((s) => s.snapshot);
  const snapshot = given === undefined ? stored : given;
  const { rescued, bad } = useImage((s) => s.tiles);
  const cells = useMemo(
    () => (snapshot && snapshot.size > 0 ? cellsFromBlocks(snapshot.blocks, snapshot.size, CELLS) : EMPTY),
    [snapshot],
  );

  return (
    <Panel title="Drive health map">
      <div className="map" role="img" aria-label={!snapshot ? "Drive health map: no data yet" : given ? "Drive health map of the opened map file" : `Drive health map: ${rescued} rescued, ${bad} bad`}>
        {cells.map((c, i) => (
          <i key={i} className={c} />
        ))}
      </div>
      <div className="legend">
        <span className="tone-green">Rescued</span>
        <span className="tone-amber">Needs retry</span>
        <span className="tone-red">Bad sector</span>
        <span className="tone-grey">Not tried yet</span>
      </div>
      <p className="dim small">Each square is an equal slice of the drive; its colour is the worst thing found in that slice.</p>
    </Panel>
  );
}
