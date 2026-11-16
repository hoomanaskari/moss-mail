import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { start, tryAgain, useStartup } from "./startup";

const s = () => useStartup.getState();
const pristine = useStartup.getState();

beforeEach(() => {
  useStartup.setState(pristine, true);
  vi.spyOn(console, "error").mockImplementation(() => {});
});
afterEach(() => vi.restoreAllMocks());

describe("startup", () => {
  it("loads the mail, and nothing stands in front of it once it has", async () => {
    const connect = vi.fn(async () => {});
    await start(connect);
    expect(connect).toHaveBeenCalledOnce();
    expect(s()).toEqual({ failed: undefined, trying: false });
    expect(console.error).not.toHaveBeenCalled();
  });

  it("when the mail can't be loaded, says why instead of showing any", async () => {
    const problem = new Error("database is locked");
    await start(async () => {
      throw problem;
    });
    expect(s()).toEqual({ failed: "database is locked", trying: false });
    expect(console.error).toHaveBeenCalledWith("Moss: couldn’t load mail", problem);
  });

  it("Try again loads it again, showing it is trying, and the mail shows once it works", async () => {
    let fail = true;
    const connect = vi.fn(async () => {
      if (fail) throw "The engine stopped";
    });
    await start(connect);
    expect(s().failed).toBe("The engine stopped");
    fail = false;
    const again = tryAgain();
    expect(s()).toEqual({ failed: "The engine stopped", trying: true });
    await again;
    expect(connect).toHaveBeenCalledTimes(2);
    expect(s()).toEqual({ failed: undefined, trying: false });
  });

  it("Try again before anything was loaded has nothing to load and shows the mail", async () => {
    vi.resetModules();
    const fresh = await import("./startup");
    fresh.useStartup.setState({ failed: "database is locked" });
    await fresh.tryAgain();
    expect(fresh.useStartup.getState().failed).toBeUndefined();
  });
});
