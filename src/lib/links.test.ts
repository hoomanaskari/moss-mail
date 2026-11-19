import { afterEach, describe, expect, it, vi } from "vitest";
import { useStore } from "../state/store";
import { resetStore } from "../test/fixtures";
import { openLink } from "./links";

afterEach(() => vi.restoreAllMocks());

describe("openLink", () => {
  it("opens web links in a new browser tab that can't reach back into Moss", () => {
    const open = vi.spyOn(window, "open").mockReturnValue(null);
    openLink("https://northwind.co/agenda");
    expect(open).toHaveBeenCalledWith("https://northwind.co/agenda", "_blank", "noopener,noreferrer");
  });

  it("opens mail links as a new email in Moss, not in another app", async () => {
    resetStore([]);
    const open = vi.spyOn(window, "open").mockReturnValue(null);
    openLink("mailto:lena@northwind.co?subject=Hello");
    await vi.waitFor(() => expect(useStore.getState().draft?.subject).toBe("Hello"));
    expect(useStore.getState().draft!.to.map((p) => p.email)).toEqual(["lena@northwind.co"]);
    expect(open).not.toHaveBeenCalled();
  });

  it("refuses anything that isn't a web or mail link", () => {
    const open = vi.spyOn(window, "open").mockReturnValue(null);
    openLink("javascript:alert(1)");
    openLink("file:///etc/passwd");
    expect(open).not.toHaveBeenCalled();
  });
});
