import { expect, it, vi } from "vitest";
import { ai, demoAi, setAi, type AiStatus } from "../services/ai";
import { useAiSettings, watchAi } from "./aiSettings";

const s = () => useAiSettings.getState();

it("loads the real status from the backend", async () => {
  expect(s().status).toBeUndefined();
  await s().load();
  expect(s().status?.machine).toBe("M3, 16 GB memory");
  expect(s().status?.writer).toBe("e4b");
});

it("chooses a model that fits; one that needs more memory is refused and says so", async () => {
  await s().load();
  await s().chooseModel("e2b");
  expect(s().status?.writer).toBe("e2b");
  await s().chooseModel("26b");
  expect(s().status?.writer).toBe("e2b");
  expect(s().problem).toBe("That model needs more memory than this Mac has (26b).");
  await s().chooseModel("e4b");
  expect(s().problem).toBeUndefined();
});

it("downloads and pauses through the backend", async () => {
  const demo = demoAi({ state: "downloading", done: 2, total: 10 });
  setAi(demo);
  const cancel = vi.spyOn(demo, "cancel");
  await s().load();
  await s().cancel();
  expect(cancel).toHaveBeenCalledOnce();
  await s().download();
  expect(s().status?.download).toEqual({ state: "ready" });
});

it("a backend error becomes a sentence on screen", async () => {
  setAi({ ...ai(), download: () => Promise.reject("The disk is full.") });
  await s().load();
  await s().download();
  expect(s().problem).toBe("The disk is full.");
});

it("gets and stops one model on its own through the backend", async () => {
  const demo = demoAi({ models: { e2b: { state: "downloading", done: 3 } } });
  setAi(demo);
  const stop = vi.spyOn(demo, "stopModel");
  await s().load();
  await s().stopModel("e2b");
  expect(stop).toHaveBeenCalledWith("e2b");
  expect(s().status?.writers[0].download).toEqual({ state: "missing", done: 3 });
  await s().downloadModels(["e2b"]);
  expect(s().status?.writers[0].download).toEqual({ state: "ready" });
});

it("follows status changes while watched", async () => {
  const demo = demoAi();
  setAi(demo);
  const stop = watchAi();
  await demo.download();
  expect(s().status?.download).toEqual({ state: "ready" });
  stop();
  await demo.choose("e2b");
  expect((s().status as AiStatus).writer).toBe("e4b");
});

it("picks a plan, loads the status alone and removes the models through the backend", async () => {
  setAi(demoAi());
  await s().loadStatus();
  expect(s().status?.plan).toBeNull();
  await s().choosePlan("essentials");
  expect(s().status?.plan).toBe("essentials");
  expect(s().status?.features).toEqual({ search: true, sorting: true, writing: false });
  await s().removeModel("decider");
  expect(s().status?.features.sorting).toBe(false);
  await s().remove();
  expect(s().status?.plan).toBe("skip");
  expect(s().status?.onDisk).toBe(0);
});

it("a chosen model shows as chosen at once, and goes back if the backend refuses it", async () => {
  await s().load();
  let refuse: (error: unknown) => void = () => {};
  setAi({ ...ai(), choose: () => new Promise<AiStatus>((_, r) => (refuse = r)) });
  const choosing = s().chooseModel("e2b");
  expect(s().status?.writer).toBe("e2b");
  refuse("The disk is full.");
  await choosing;
  expect(s().status?.writer).toBe("e4b");
  expect(s().problem).toBe("The disk is full.");
  // One that needs more memory never shows as chosen.
  void s().chooseModel("26b");
  expect(s().status?.writer).toBe("e4b");
});
