/** Maximum scroll-spacer height. Browsers cap element height (~17M-33M px); stay well under. */
export const MAX_SPACER = 10_000_000;

export interface VWindow {
  /** Index of the first row to render. */
  first: number;
  /** Number of rows to render. */
  count: number;
  /** Pixel offset of row `first` relative to the top of the viewport (usually <= 0). */
  offset: number;
  /** Height of the scroll spacer. */
  spacer: number;
}

/**
 * Which rows to draw for a scroll position. The rows are drawn in a container that
 * stays pinned to the viewport (position: sticky), shifted by `offset`.
 * When total*rowHeight exceeds MAX_SPACER the scrollbar is compressed: scrollTop is
 * scaled up so the whole list is still reachable, and the last row lines up with
 * the bottom edge at maximum scroll.
 */
export function windowAt(scrollTop: number, viewport: number, rowHeight: number, total: number, overscan = 4): VWindow {
  const full = total * rowHeight;
  const spacer = Math.min(full, MAX_SPACER);
  if (total <= 0 || viewport <= 0) return { first: 0, count: 0, offset: 0, spacer };
  const maxScroll = Math.max(0, spacer - viewport);
  const top = Math.min(Math.max(0, scrollTop), maxScroll);
  const scale = full > spacer && maxScroll > 0 ? (full - viewport) / maxScroll : 1;
  const virtualTop = top * scale;
  const firstVisible = Math.floor(virtualTop / rowHeight);
  const first = Math.max(0, Math.min(total - 1, firstVisible - overscan));
  const count = Math.min(total - first, Math.ceil(viewport / rowHeight) + 2 * overscan + 1);
  return { first, count, offset: first * rowHeight - virtualTop, spacer };
}
