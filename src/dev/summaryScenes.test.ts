import { act } from "@testing-library/react";
import { beforeEach, expect, it, vi } from "vitest";
import { summaries } from "../services/summaries";
import { useStore } from "../state/store";
import { useSummary } from "../state/summary";
import { resetDemoStore } from "../test/fixtures";
import { summaryScenes } from "./summaryScenes";

beforeEach(() => {
  resetDemoStore();
  useSummary.setState({ threadId: "q4-budget", run: 0, phase: "none", menuOpen: false });
});

/** Applies a scene and lets its delayed step run, as the open email would. */
async function apply(name: string) {
  vi.useFakeTimers();
  summaryScenes[name].apply();
  await act(() => vi.advanceTimersByTimeAsync(60));
  vi.useRealTimers();
}

it("each opens the Q4 budget email with its summaries", async () => {
  await apply("summary-detailed");
  expect(useStore.getState().openThreadId).toBe("q4-budget");
  expect((await summaries().kept("q4-budget")).summary?.kind).toBe("detailed");
  await apply("summary-facts-domain");
  expect((await summaries().kept("q4-budget")).rule?.sender).toBe("northwind.co");
});

it("the menu, a summary you asked for, and the states", async () => {
  await apply("summary-menu");
  expect(useSummary.getState().menuOpen).toBe(true);
  await apply("summary-short");
  await vi.waitFor(() => expect(useSummary.getState().shown?.kind).toBe("short"));
  await apply("summary-loading");
  expect(useSummary.getState().phase).toBe("loading");
  await apply("summary-no-model");
  await vi.waitFor(() => expect(useSummary.getState().phase).toBe("notReady"));
  await apply("summary-failed");
  await vi.waitFor(() => expect(useSummary.getState().phase).toBe("failed"));
});

it("settings open on AI, where Summaries has just its switch", () => {
  summaryScenes["settings-ai-summaries"].apply();
  expect(useStore.getState().view).toBe("settings");
});
