import { useVirtualizer } from "@tanstack/react-virtual";
import { useEffect, useRef, type ReactNode } from "react";
import { cx } from "./ui";

interface Props {
  count: number;
  rowHeight: number;
  /** Renders one row's content; only rows in view are rendered. */
  renderRow: (index: number) => ReactNode;
  label: string;
  empty: string;
  /** Changing this scrolls back to the top (a new folder, new filters). */
  resetKey?: string;
  short?: boolean;
}

/** A fixed-row-height list that stays smooth with millions of rows. */
export function VirtualList({ count, rowHeight, renderRow, label, empty, resetKey, short }: Props) {
  const scroller = useRef<HTMLDivElement>(null);
  const v = useVirtualizer({
    count,
    getScrollElement: () => scroller.current,
    estimateSize: () => rowHeight,
    overscan: 12,
    // Used before the first measurement (and in tests, where nothing has a size).
    initialRect: { width: 800, height: short ? 200 : 480 },
  });

  useEffect(() => {
    v.scrollToOffset(0);
  }, [resetKey, v]);

  return (
    <div className={cx("listbox", short && "short")}>
      <div ref={scroller} className="vl" role="list" aria-label={label}>
        <div className="vl-space" style={{ height: v.getTotalSize() }}>
          {v.getVirtualItems().map((item) => (
            <div key={item.key} role="listitem" className="vl-row" style={{ height: rowHeight, transform: `translateY(${item.start}px)` }}>
              {renderRow(item.index)}
            </div>
          ))}
        </div>
      </div>
      {count === 0 && <div className="vl-empty">{empty}</div>}
    </div>
  );
}
