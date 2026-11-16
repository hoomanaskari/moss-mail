import { afterEach, expect, it, vi } from "vitest";
import { readRemoteImagePolicy, useRemoteImages } from "./remoteImages";
const key = "moss.remoteImages";
afterEach(() => { vi.restoreAllMocks(); localStorage.clear(); useRemoteImages.setState({ policy: "familiar", saved: true }); });
it("preserves familiar loading by default and restores either explicit choice", () => {
  localStorage.removeItem(key);
  expect(readRemoteImagePolicy()).toEqual({ policy: "familiar", saved: true });
  for (const policy of ["ask", "familiar"] as const) {
    useRemoteImages.getState().setPolicy(policy);
    expect(localStorage.getItem(key)).toBe(policy);
    expect(readRemoteImagePolicy()).toEqual({ policy, saved: true });
    expect(useRemoteImages.getState()).toMatchObject({ policy, saved: true });
  }
});
it("asks before loading when saved policy is unreadable", () => {
  localStorage.setItem(key, "unexpected");
  expect(readRemoteImagePolicy()).toEqual({ policy: "ask", saved: false });
  vi.spyOn(Storage.prototype, "getItem").mockImplementation(() => { throw Error("unavailable"); });
  expect(readRemoteImagePolicy()).toEqual({ policy: "ask", saved: false });
});
it("applies the choice but reports when it cannot be remembered", () => {
  vi.spyOn(Storage.prototype, "setItem").mockImplementation(() => { throw Error("unavailable"); });
  useRemoteImages.getState().setPolicy("ask");
  expect(useRemoteImages.getState()).toMatchObject({ policy: "ask", saved: false });
});
