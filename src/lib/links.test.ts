import { afterEach, describe, expect, it, vi } from "vitest";
import { openLink } from "./links";

afterEach(() => vi.restoreAllMocks());

describe("openLink", () => {
  it("opens web links in a new browser tab that can't reach back into Moss", () => {
    const open = vi.spyOn(window, "open").mockReturnValue(null);
    openLink("https://northwind.co/agenda");
    expect(open).toHaveBeenCalledWith("https://northwind.co/agenda", "_blank", "noopener,noreferrer");
  });

  it("hands mailto links to the browser too", () => {
    const open = vi.spyOn(window, "open").mockReturnValue(null);
    openLink("mailto:lena@northwind.co");
    expect(open).toHaveBeenCalledOnce();
  });

  it("refuses anything that isn't a web or mail link", () => {
    const open = vi.spyOn(window, "open").mockReturnValue(null);
    openLink("javascript:alert(1)");
    openLink("file:///etc/passwd");
    expect(open).not.toHaveBeenCalled();
  });
});
