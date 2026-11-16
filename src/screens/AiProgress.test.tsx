import { act, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { expect, it } from "vitest";
import { demoAi, type AiStatus } from "../services/ai";
import { useAiSettings } from "../state/aiSettings";
import { useStore } from "../state/store";
import { aiProgress, AiProgress, meaningHint } from "./AiProgress";

async function statusOf(start: Parameters<typeof demoAi>[0]) {
  return demoAi(start).status();
}

const downloading = { state: "downloading", done: 2_100_000_000, total: 6_400_000_000 } as const;

it("says how far the AI download is, and why it waits", async () => {
  expect(aiProgress(undefined)).toBeUndefined();
  expect(aiProgress(await statusOf({}))).toBeUndefined();
  expect(aiProgress(await statusOf({ download: downloading }))).toEqual({ label: "Getting AI · 2.1 of 6.4 GB", value: 2.1 / 6.4, paused: false });
  const searchOn = await statusOf({ download: downloading, features: { search: true, sorting: false, writing: false } });
  expect(aiProgress(searchOn)?.label).toBe("Search by meaning is on · 2.1 of 6.4 GB");
  expect(aiProgress(await statusOf({ download: downloading, paused: "battery" }))).toMatchObject({ label: "AI paused · battery below 20%", paused: true });
  expect(aiProgress(await statusOf({ download: downloading, paused: "hotspot" }))?.label).toBe("AI paused · phone hotspot");
  expect(aiProgress(await statusOf({ download: { state: "downloading", done: 0, total: 0 } }))?.value).toBe(0);
});

it("shows in the top bar while downloading, and opens Settings → AI", async () => {
  const { rerender } = render(<AiProgress />);
  expect(screen.queryByRole("progressbar")).toBeNull();
  const status: AiStatus = await statusOf({ download: downloading });
  act(() => useAiSettings.setState({ status }));
  rerender(<AiProgress />);
  await userEvent.setup().click(screen.getByRole("button", { name: /Getting AI/ }));
  expect(useStore.getState()).toMatchObject({ view: "settings", settingsSection: "ai" });
});

it("hints at search by meaning only when its model isn't here or coming", async () => {
  expect(meaningHint(undefined)).toBeUndefined();
  expect(meaningHint(await statusOf({ plan: "skip" }))).toBe("1.4 GB");
  expect(meaningHint(await statusOf({ download: { state: "ready" } }))).toBeUndefined();
  expect(meaningHint(await statusOf({ download: downloading }))).toBeUndefined();
  expect(meaningHint({ ...(await statusOf({})), plans: [] })).toBeUndefined();
});
