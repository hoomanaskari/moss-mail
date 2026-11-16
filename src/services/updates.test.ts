import { afterEach, expect, it } from "vitest";
import { demoUpdates, setUpdates, updates } from "./updates";

afterEach(() => setUpdates(demoUpdates()));

it("the demo is Moss 0.1.0 with nothing newer", async () => {
  const demo = demoUpdates();
  expect(await demo.version()).toBe("0.1.0");
  expect(await demo.check()).toBeNull();
});

it("the demo can offer a newer Moss, count its download and restart into it", async () => {
  const demo = demoUpdates({ next: "0.2.0" });
  expect(await demo.check()).toEqual({ version: "0.2.0" });
  const seen: [number, number][] = [];
  await demo.install((done, total) => seen.push([done, total]));
  expect(seen).toEqual([
    [5, 10],
    [10, 10],
  ]);
  await demo.restart();
  expect(await demo.version()).toBe("0.2.0");
  expect(await demo.check()).toBeNull();
});

it("installing with nothing found is a mistake the demo reports", async () => {
  await expect(demoUpdates().install(() => {})).rejects.toThrow("No update to install");
});

it("the desktop app swaps in its own updater", () => {
  const own = demoUpdates({ version: "9.9.9" });
  setUpdates(own);
  expect(updates()).toBe(own);
});

it("restarting with nothing new keeps the same Moss", async () => {
  const demo = demoUpdates();
  await demo.restart();
  expect(await demo.version()).toBe("0.1.0");
});
