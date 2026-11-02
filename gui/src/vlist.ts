import { windowAt } from "./virtual.ts";

/**
 * Virtualized list: only the rows in (and just around) the viewport exist in the DOM,
 * so a million entries cost the same as fifty. Rows are pooled and re-rendered by
 * index; `render` must escape anything it puts in innerHTML.
 */
export class VList {
  /** Container of the row elements; attach click/change listeners here (rows carry data-i). */
  readonly rows: HTMLElement;
  private viewport: HTMLElement;
  private spacer: HTMLElement;
  private empty: HTMLElement;
  private pool: HTMLElement[] = [];
  private total = 0;
  private height = 0;
  private raf = 0;
  private ro: ResizeObserver;

  constructor(host: HTMLElement, private rowHeight: number, private render: (i: number, row: HTMLElement) => void, label: string) {
    host.replaceChildren();
    this.viewport = document.createElement("div");
    this.viewport.className = "vl";
    this.viewport.tabIndex = 0;
    this.viewport.setAttribute("role", "list");
    this.viewport.setAttribute("aria-label", label);
    this.spacer = document.createElement("div");
    this.rows = document.createElement("div");
    this.rows.className = "vl-rows";
    this.empty = document.createElement("div");
    this.empty.className = "vl-empty";
    this.spacer.append(this.rows);
    this.viewport.append(this.spacer);
    host.append(this.viewport, this.empty);
    this.viewport.addEventListener("scroll", () => this.schedule(), { passive: true });
    this.ro = new ResizeObserver(() => this.schedule());
    this.ro.observe(this.viewport);
  }

  setCount(n: number) { this.total = n; this.update(true); }
  /** Re-render the visible rows (data changed, count did not). */
  refresh() { this.update(true); }
  /** Text shown in place of rows when the list is empty. */
  setEmpty(text: string) { this.empty.textContent = text; }
  scrollToTop() { this.viewport.scrollTop = 0; }
  destroy() { this.ro.disconnect(); cancelAnimationFrame(this.raf); }

  private schedule() { if (!this.raf) this.raf = requestAnimationFrame(() => { this.raf = 0; this.update(false); }); }

  private update(force: boolean) {
    this.height = this.viewport.clientHeight;
    const w = windowAt(this.viewport.scrollTop, this.height, this.rowHeight, this.total);
    this.spacer.style.height = `${w.spacer}px`;
    this.rows.style.height = `${this.height}px`;
    this.empty.hidden = this.total > 0;
    while (this.pool.length < w.count) {
      const el = document.createElement("div");
      el.className = "vl-row";
      el.setAttribute("role", "listitem");
      el.style.height = `${this.rowHeight}px`;
      this.pool.push(el);
      this.rows.append(el);
    }
    while (this.pool.length > w.count) this.pool.pop()!.remove();
    this.rows.style.transform = `translateY(${w.offset}px)`;
    for (let k = 0; k < w.count; k++) {
      const el = this.pool[k], i = w.first + k;
      if (force || el.dataset.i !== String(i)) { el.dataset.i = String(i); this.render(i, el); }
    }
  }
}
