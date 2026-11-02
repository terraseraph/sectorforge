import "@testing-library/jest-dom/vitest";
import { cleanup } from "@testing-library/react";
import { afterEach } from "vitest";

afterEach(cleanup);

// jsdom does no layout, so every element measures 0x0 and a virtual list would
// render no rows. Give list viewports (TanStack reads clientWidth/clientHeight) a fixed size.
for (const [prop, size] of [["clientWidth", 800], ["clientHeight", 480]] as const) {
  const real = Object.getOwnPropertyDescriptor(Element.prototype, prop);
  Object.defineProperty(HTMLElement.prototype, prop, {
    configurable: true,
    get(this: HTMLElement) {
      return this.classList.contains("vl") ? size : (real?.get?.call(this) ?? 0);
    },
  });
}
