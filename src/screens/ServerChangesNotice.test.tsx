import { act, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { resetStore, work, personal } from "../test/fixtures";
import { fakeMail } from "../test/fakeMail";
import { useStore } from "../state/store";
import { resetServerChanges, useServerChanges } from "../services/serverChanges";
import type { ServerChangeStatus } from "../services/mail";
import { ServerChangesNotice, slowChangeMs } from "./ServerChangesNotice";

const status: ServerChangeStatus = { accountId: "work", change: "0123456789abcdef0123456789abcdef", revision: 4, state: "blocked", failure: "uncertain", kind: "move", completed: 1, pending: 2, discarded: 0, dropped: 0 };
beforeEach(() => { resetStore([]); useStore.setState({ accounts: [work, personal] }); resetServerChanges(); fakeMail(); });
afterEach(() => vi.useRealTimers());
const onItsWay = (overrides: Partial<ServerChangeStatus> = {}) =>
  useServerChanges.setState({ byAccount: { work: { status: { ...status, state: "claimed", failure: null, kind: "draftCleanup", ...overrides }, unavailable: false } } });
it("says nothing about a change on its way until it has taken a few seconds: most reach the server at once", () => {
  vi.useFakeTimers();
  onItsWay();
  render(<ServerChangesNotice />);
  expect(screen.queryByRole("status")).not.toBeInTheDocument();
  act(() => vi.advanceTimersByTime(slowChangeMs - 1));
  expect(screen.queryByRole("status")).not.toBeInTheDocument();
  act(() => vi.advanceTimersByTime(1));
  expect(screen.getByRole("status")).toHaveTextContent("Work: Draft cleanup is waiting");
});
it("never shows a change that reached the server in time, and times the next one from its own start", () => {
  vi.useFakeTimers();
  onItsWay();
  render(<ServerChangesNotice />);
  act(() => vi.advanceTimersByTime(slowChangeMs / 2));
  act(() => useServerChanges.setState({ byAccount: { work: { status: null, unavailable: false } } }));
  act(() => vi.advanceTimersByTime(slowChangeMs));
  expect(screen.queryByRole("status")).not.toBeInTheDocument();
  act(() => onItsWay());
  act(() => vi.advanceTimersByTime(slowChangeMs - 1));
  expect(screen.queryByRole("status")).not.toBeInTheDocument();
  act(() => vi.advanceTimersByTime(1));
  expect(screen.getByRole("status")).toBeInTheDocument();
});
it("shows a stuck change at once, and a failed status read, with no wait", () => {
  vi.useFakeTimers();
  useServerChanges.setState({ byAccount: { work: { status, unavailable: false }, personal: { status: { ...status, accountId: "personal", state: "claimed", failure: null }, unavailable: true } } });
  render(<ServerChangesNotice />);
  expect(screen.getAllByRole("status")).toHaveLength(2);
});
it("shows retained partial work without claiming that the pending parts succeeded", () => {
  useServerChanges.setState({ byAccount: { work: { status, unavailable: false } } });
  render(<ServerChangesNotice />);
  expect(screen.getByRole("status")).toHaveTextContent("Work: Move is waiting");
  expect(screen.getByText(/The server acknowledged 1 of 3 parts/)).toBeInTheDocument();
  expect(screen.getByText(/may mean this change partly worked/)).toBeInTheDocument();
  expect(screen.queryByRole("button", { name: /send/i })).not.toBeInTheDocument();
});
it.each(["refused", "uncertain"] as const)("tries a %s change again only when you ask, then checks its status", async (failure) => {
  const { backend } = fakeMail();
  useServerChanges.setState({ byAccount: { work: { status: { ...status, failure }, unavailable: false } } });
  render(<ServerChangesNotice />);
  expect(backend.retryServerChange).not.toHaveBeenCalled();
  await userEvent.click(screen.getByRole("button", { name: "Try the server change again in Work" }));
  expect(backend.retryServerChange).toHaveBeenCalledWith("work", status.change, status.revision);
  expect(backend.discardServerChange).not.toHaveBeenCalled();
  expect(backend.serverChangeStatus).toHaveBeenCalledWith("work");
  expect(screen.queryByRole("status")).not.toBeInTheDocument();
});
it.each(["refused", "uncertain", "unreadable", "tooLarge"] as const)("discards a %s change, saying it stays on this Mac only", async (failure) => {
  const { backend } = fakeMail();
  useServerChanges.setState({ byAccount: { work: { status: { ...status, failure }, unavailable: false } } });
  render(<ServerChangesNotice />);
  expect(screen.getByRole("status")).toHaveTextContent("Discard stops Moss trying: the change stays on this Mac only, and your mail server keeps your mail as it was.");
  expect(!!screen.queryByRole("button", { name: /Try the server change again/ })).toBe(failure === "refused" || failure === "uncertain");
  await userEvent.click(screen.getByRole("button", { name: "Discard the server change in Work" }));
  expect(backend.discardServerChange).toHaveBeenCalledWith("work", status.change, status.revision);
  expect(backend.serverChangeStatus).toHaveBeenCalledWith("work");
});
it("says when trying again or discarding didn't work, and keeps the change", async () => {
  const { backend } = fakeMail({
    retryServerChange: vi.fn(async () => { throw { kind: "invalid", message: "This server change has changed since you saw it. Check its status again." }; }),
    serverChangeStatus: vi.fn(async () => status),
  });
  useServerChanges.setState({ byAccount: { work: { status, unavailable: false } } });
  render(<ServerChangesNotice />);
  await userEvent.click(screen.getByRole("button", { name: "Try the server change again in Work" }));
  expect(screen.getByRole("status")).toHaveTextContent("Couldn’t do that: This server change has changed since you saw it. Check its status again.");
  expect(backend.serverChangeStatus).toHaveBeenCalledWith("work");
});
it("offers nothing to repeat while a change is waiting or on its way", () => {
  useServerChanges.setState({ byAccount: { work: { status: { ...status, state: "claimed", failure: null }, unavailable: false } } });
  render(<ServerChangesNotice />);
  expect(screen.queryByRole("button", { name: /Try the server change again|Discard/ })).not.toBeInTheDocument();
});
it.each(["refused", "unreadable", "tooLarge"] as const)("explains %s retained work", (failure) => {
  useServerChanges.setState({ byAccount: { work: { status: { ...status, failure }, unavailable: false } } });
  render(<ServerChangesNotice />);
  expect(screen.getByRole("status")).toHaveTextContent(failure === "refused" ? "server refused this change" : failure === "unreadable" ? "cannot safely read" : "too large");
});
it("distinguishes a claimed attempt and stopped parts from success", () => {
  vi.useFakeTimers();
  useServerChanges.setState({ byAccount: { work: { status: { ...status, state: "claimed", failure: null, discarded: 1, dropped: 1 }, unavailable: false } } });
  render(<ServerChangesNotice />);
  act(() => vi.advanceTimersByTime(slowChangeMs));
  expect(screen.getByRole("status")).toHaveTextContent("Waiting for a definite server outcome");
  expect(screen.getByText(/earlier parts were stopped/)).toBeInTheDocument();
});
it("hides ordinary waiting work and respects the selected account", () => {
  useServerChanges.setState({ byAccount: { work: { status: { ...status, state: "queued", failure: null }, unavailable: false }, personal: { status: { ...status, accountId: "personal" }, unavailable: false } } });
  useStore.setState({ accountFilter: "work" });
  render(<ServerChangesNotice />);
  expect(screen.queryByRole("status")).not.toBeInTheDocument();
});
it("shows a failed status read and checks again without repeating a server action", async () => {
  const { backend } = fakeMail();
  useServerChanges.setState({ byAccount: { work: { status: null, unavailable: true } } });
  render(<ServerChangesNotice />);
  expect(screen.getByRole("status")).toHaveTextContent("Cannot check whether server changes are waiting");
  await userEvent.click(screen.getByRole("button", { name: "Check server changes in Work" }));
  expect(backend.serverChangeStatus).toHaveBeenCalledWith("work");
  expect(backend.syncNow).not.toHaveBeenCalled();
  expect(screen.queryByRole("status")).not.toBeInTheDocument();
});
it("retains the last known state during status-read failures and removes bidi controls from labels", () => {
  useStore.setState({ accounts: [{ ...work, name: "Wo\u202Erk" }] });
  useServerChanges.setState({ byAccount: { work: { status, unavailable: true } } });
  render(<ServerChangesNotice />);
  expect(screen.getByRole("status")).toHaveTextContent("Work: Move is waiting");
  expect(screen.getByRole("status")).toHaveTextContent("Last known status is shown");
  expect(screen.getByRole("status").textContent).not.toContain("\u202E");
});
it("preserves a last-known queued status when its refresh is unavailable", () => {
  useServerChanges.setState({ byAccount: { work: { status: { ...status, state: "queued", failure: null, discarded: 0, dropped: 1 }, unavailable: true } } });
  render(<ServerChangesNotice />);
  expect(screen.getByRole("status")).toHaveTextContent("This change is waiting to reach the server");
  expect(screen.getByRole("status")).toHaveTextContent("earlier parts were stopped");
});
