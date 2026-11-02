export type Status = "finished" | "non_trimmed" | "non_scraped" | "bad_sector" | "non_tried";
export interface Block { pos: number; size: number; status: Status }

// Higher = shown in preference when a cell covers several statuses.
// Amber/red outrank grey so damage is never hidden by untried neighbours.
const RANK: Record<Status, number> = { finished: 0, non_tried: 1, non_trimmed: 2, non_scraped: 2, bad_sector: 3 };

/** Colour class per cell: each cell is an equal slice of [0,size); worst status wins. */
export function cellsFromBlocks(blocks: Block[], size: number, n: number): Status[] {
  const cells: (Status | null)[] = new Array(n).fill(null);
  for (const b of blocks) {
    if (b.size <= 0) continue;
    const a = Math.floor((b.pos * n) / size);
    const z = Math.min(n - 1, Math.floor(((b.pos + b.size - 1) * n) / size));
    for (let i = a; i <= z; i++) {
      const c = cells[i];
      if (c === null || RANK[b.status] > RANK[c]) cells[i] = b.status;
    }
  }
  return cells.map((c) => c ?? "non_tried");
}
