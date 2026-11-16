import { act, renderHook } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { resetStore } from "../test/fixtures";
import { useStore } from "./store";

const setAppIcon = vi.fn(async () => {});
vi.mock("../lib/appIcon", async (actual) => ({ ...(await actual<typeof import("../lib/appIcon")>()), setAppIcon }));

const { useAppIcon } = await import("./useAppIcon");

beforeEach(() => {
  resetStore([]);
  setAppIcon.mockClear();
});

describe("useAppIcon", () => {
  it("leaves the Dock icon to the Mac while Moss follows the system", () => {
    useStore.getState().setTheme("system");
    renderHook(() => useAppIcon());
    expect(setAppIcon).toHaveBeenLastCalledWith("system");
  });

  it("shows Moss's theme while it is fixed, and the Mac's again on System", () => {
    useStore.getState().setTheme("dark");
    renderHook(() => useAppIcon());
    expect(setAppIcon).toHaveBeenLastCalledWith("dark");
    act(() => useStore.getState().setTheme("light"));
    expect(setAppIcon).toHaveBeenLastCalledWith("light");
    act(() => useStore.getState().setTheme("system"));
    expect(setAppIcon).toHaveBeenLastCalledWith("system");
  });

  it("asks only when the icon changes", () => {
    useStore.getState().setTheme("dark");
    renderHook(() => useAppIcon());
    act(() => useStore.setState({ readingSize: "larger" }));
    expect(setAppIcon).toHaveBeenCalledOnce();
  });
});
