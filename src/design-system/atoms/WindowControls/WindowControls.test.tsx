import { render } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { WindowControls } from "./WindowControls";

// The platform flags are read once at import time; getters let each test pick one.
const platform = vi.hoisted(() => ({ isMac: true, isTauri: false }));
vi.mock("../../../lib/platform", () => ({
  get isMac() {
    return platform.isMac;
  },
  get isTauri() {
    return platform.isTauri;
  },
}));

beforeEach(() => {
  platform.isMac = true;
  platform.isTauri = false;
});

describe("WindowControls", () => {
  it("draws three traffic lights in a browser tab", () => {
    platform.isMac = false;
    const { container } = render(<WindowControls />);
    const dots = container.firstElementChild!;
    expect(dots).toHaveAttribute("aria-hidden", "true");
    expect([...dots.children].map((d) => (d as HTMLElement).style.background)).toEqual([
      "rgb(232, 105, 94)",
      "rgb(233, 185, 73)",
      "rgb(108, 192, 85)",
    ]);
  });

  it("only reserves space for the real ones in the macOS app", () => {
    platform.isTauri = true;
    const { container } = render(<WindowControls />);
    const reserve = container.firstElementChild!;
    expect(reserve).toHaveAttribute("aria-hidden", "true");
    expect(reserve.children).toHaveLength(0);
  });

  it("renders nothing in the Windows and Linux app", () => {
    platform.isTauri = true;
    platform.isMac = false;
    const { container } = render(<WindowControls />);
    expect(container).toBeEmptyDOMElement();
  });
});
