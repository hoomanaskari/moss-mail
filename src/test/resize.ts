import { vi } from "vitest";

/** Layout supplied explicitly by tests. jsdom itself has no layout engine. */
export class TestResizeObserver implements ResizeObserver {
  static instances = new Set<TestResizeObserver>();
  private elements = new Set<Element>();
  constructor(private callback: ResizeObserverCallback) {}
  observe(element: Element) {
    this.elements.add(element);
    TestResizeObserver.instances.add(this);
  }
  unobserve(element: Element) { this.elements.delete(element); }
  disconnect() {
    this.elements.clear();
    TestResizeObserver.instances.delete(this);
  }
  static resize(element: Element, height: number, viewport = false) {
    Object.defineProperty(element, "getBoundingClientRect", { configurable: true, value: () => new DOMRect(0, 0, 1000, height) });
    if (viewport) Object.defineProperty(element, "clientHeight", { configurable: true, value: height });
    for (const observer of [...this.instances]) {
      if (observer.elements.has(element)) observer.callback([], observer);
    }
  }
}

/** Supply layout for rulers on their very first commit, before observers attach. */
export function mockRulerHeights(heights: Record<string, number>) {
  return vi.spyOn(Element.prototype, "getBoundingClientRect").mockImplementation(function (this: Element) {
    return new DOMRect(0, 0, 1000, heights[this.getAttribute("data-testid") ?? ""] ?? 0);
  });
}
