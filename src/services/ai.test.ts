import { expect, it } from "vitest";
import { ai, demoAi, demoApple, demoWriters, type AiStatus, setAi, takesOver, totalFor } from "./ai";

it("the demo starts on the Mac in the design, recommending E4B, nothing downloaded", async () => {
  const status = await demoAi().status();
  expect(status.machine).toBe("M3, 16 GB memory");
  expect(status.writer).toBe("e4b");
  expect(status.deciderSize).toBe("About 1.1 GB");
  expect(status.download).toEqual({ state: "missing", done: 0, total: totalFor(demoWriters[1]) });
  expect(status.writers.find((w) => w.tooBig)?.id).toBe("26b");
});

it("the demo downloads at once, switches models, and can start in any state", async () => {
  const demo = demoAi();
  const heard: string[] = [];
  const stop = demo.onStatus((s) => heard.push(s.download.state));
  expect((await demo.download()).download).toEqual({ state: "ready" });
  expect(heard).toEqual(["ready"]);
  stop();
  await demo.download();
  expect(heard).toEqual(["ready"]);

  const chosen = await demo.choose("e2b");
  expect(chosen.writer).toBe("e2b");
  const essentials = totalFor(demoWriters[0]) - demoWriters[0].bytes;
  expect(chosen.download).toEqual({ state: "missing", done: essentials, total: totalFor(demoWriters[0]) });
  expect(chosen.features).toEqual({ search: true, sorting: true, writing: false });
  await expect(demo.choose("26b")).rejects.toMatchObject({ kind: "choice" });

  const failed = demoAi({ state: "failed", done: 5, total: 10, message: "No network." });
  expect((await failed.status()).download.state).toBe("failed");
  await failed.cancel();
  expect((await failed.status()).download).toEqual({ state: "missing", done: 5, total: 10 });
  const ready = demoAi({ state: "ready" });
  await ready.cancel();
  expect((await ready.status()).download).toEqual({ state: "ready" });
});

it("on a demo Mac with Apple Intelligence it writes, and a full download gets no writing model", async () => {
  const demo = demoAi({ apple: true });
  const status = await demo.status();
  expect(status.writer).toBe("apple");
  expect(status.writers.map((w) => [w.id, w.recommended])).toEqual([["apple", true], ["e2b", false], ["e4b", false], ["26b", false]]);
  expect(status.features).toEqual({ search: false, sorting: false, writing: true });
  const essentials = totalFor(demoApple);
  expect(status.plans.find((p) => p.plan === "recommended")?.bytes).toBe(essentials);
  const got = await demo.download();
  expect(got.download).toEqual({ state: "ready" });
  expect(got.downloaded).toEqual(["embedder", "decider"]);
  // A Gemma model asked for doesn't take over from it unless you asked for that one.
  expect((await demo.downloadModels(["e2b"])).writer).toBe("e2b");
  expect((await demo.choose("apple")).writer).toBe("apple");
  await expect(demo.removeModel("apple")).rejects.toMatchObject({ kind: "choice" });
  // Without Apple Intelligence there's no such model.
  await expect(demoAi().choose("apple")).rejects.toMatchObject({ kind: "choice" });
});

it("the demo writes no asks", async () => {
  expect(await demoAi().ask("1")).toBeNull();
});

it("the app can swap in another backend", () => {
  const other = demoAi({ state: "ready" });
  setAi(other);
  expect(ai()).toBe(other);
});

it("sizes read like the Rust side's", async () => {
  const { sizeLabel } = await import("./ai");
  expect(sizeLabel(16_947_541_728)).toBe("About 17 GB");
  expect(sizeLabel(6_409_244_946)).toBe("About 6.4 GB");
  expect(sizeLabel(4_977_171_584)).toBe("About 5 GB");
  expect(sizeLabel(333_590_944)).toBe("About 334 MB");
});

it("the demo plans: essentials leave writing off, and Download keeps essentials", async () => {
  const demo = demoAi();
  expect((await demo.status()).plan).toBeNull();
  await demo.plan("skip");
  expect((await demo.status()).download.state).toBe("missing");
  await demo.plan("essentials");
  const s = await demo.download();
  expect([s.plan, s.features.writing]).toEqual(["essentials", false]);
  const paused = await demoAi({ download: { state: "downloading", done: 1, total: 2 }, paused: "hotspot" }).status();
  expect(paused.paused).toBe("hotspot");
});

it("the demo keeps a writing model you switched away from until you delete it, like the Rust side", async () => {
  const demo = demoAi();
  expect((await demo.status()).downloaded).toEqual([]);
  expect((await demo.download()).downloaded).toEqual(["embedder", "decider", "e4b"]);
  await demo.choose("e2b");
  const both = await demo.download();
  expect(both.downloaded).toEqual(["embedder", "decider", "e2b", "e4b"]);
  expect((await demo.choose("e4b")).download).toEqual({ state: "ready" });
  await demo.choose("e2b");

  const s = await demo.removeModel("e4b");
  expect(s.downloaded).toEqual(["embedder", "decider", "e2b"]);
  expect(s.onDisk).toBe(both.onDisk - demoWriters[1].bytes);
  expect(s.download).toEqual({ state: "ready" });
  // The chosen one: writing stops until it's downloaded again.
  const gone = await demo.removeModel("e2b");
  expect(gone.features).toEqual({ search: true, sorting: true, writing: false });
  expect(gone.download).toEqual({ state: "missing", done: totalFor(demoWriters[0]) - demoWriters[0].bytes, total: totalFor(demoWriters[0]) });
  await demo.removeModel("decider");
  const none = await demo.removeModel("embedder");
  expect(none.features).toEqual({ search: false, sorting: false, writing: false });
  expect([none.downloaded, none.onDisk, none.download]).toEqual([[], 0, { state: "missing", done: 0, total: totalFor(demoWriters[0]) }]);
  await expect(demo.removeModel("gpt")).rejects.toMatchObject({ kind: "choice" });

  expect((await demoAi({ state: "ready" }).status()).downloaded).toEqual(["embedder", "decider", "e4b"]);
  const essentialsIn = demoAi({ download: { state: "downloading", done: 1, total: 2 }, features: { search: true, sorting: true, writing: false } });
  expect((await essentialsIn.status()).downloaded).toEqual(["embedder", "decider"]);
  const ready = demoAi({ state: "ready" });
  expect((await ready.remove()).downloaded).toEqual([]);
});

it("the demo keeps each model's own download, like the Rust side", async () => {
  const demo = demoAi({
    features: { search: true, sorting: true, writing: true },
    models: { e2b: { state: "downloading", done: 2_100_000_000 }, "26b": { state: "failed", done: 0, message: "No network." } },
  });
  const own = (s: AiStatus, id: string) => s.writers.find((w) => w.id === id)!.download;
  const s = await demo.status();
  expect([own(s, "e2b"), own(s, "e4b"), own(s, "26b")]).toEqual([
    { state: "downloading", done: 2_100_000_000 },
    { state: "ready" },
    { state: "failed", done: 0, message: "No network." },
  ]);
  expect([s.embedderDownload, s.deciderDownload]).toEqual([{ state: "ready" }, { state: "ready" }]);
  // Pausing keeps what's in; a model already stopped stays as it is.
  expect(own(await demo.stopModel("e2b"), "e2b")).toEqual({ state: "missing", done: 2_100_000_000 });
  expect(own(await demo.stopModel("26b"), "26b")).toMatchObject({ state: "failed" });
  expect(own(await demo.stopModel("e4b"), "e4b")).toEqual({ state: "ready" });
  // Downloading one makes it write; deleting leaves nothing of it.
  const got = await demo.downloadModels(["e2b"]);
  expect([own(got, "e2b"), got.writer]).toEqual([{ state: "ready" }, "e2b"]);
  expect(own(await demo.removeModel("e2b"), "e2b")).toEqual({ state: "missing", done: 0 });
  await expect(demo.downloadModels(["gpt"])).rejects.toMatchObject({ kind: "choice" });

  const other = demoAi({ have: ["e2b"], writer: "e2b" });
  const o = await other.status();
  expect([o.writer, own(o, "e2b"), o.embedderDownload]).toEqual(["e2b", { state: "ready" }, { state: "missing", done: 0 }]);
});

it("a finished model takes over when asked for or when nothing else can write, as on the Rust side", () => {
  expect(takesOver("small", "small", true)).toBe(true);
  expect(takesOver("small", null, true)).toBe(false);
  expect(takesOver("small", "big", true)).toBe(false);
  expect(takesOver("small", null, false)).toBe(true);
});

it("in the demo, a model you download writes once it's in", async () => {
  const demo = demoAi({ features: { search: true, sorting: true, writing: true } });
  expect((await demo.status()).tookOver).toBeNull();
  const s = await demo.downloadModels(["e2b"]);
  expect([s.writer, s.tookOver]).toEqual(["e2b", "e2b"]);
  expect((await demo.choose("e4b")).tookOver).toBeNull();
  // Sorting and search alone change nothing.
  expect((await demo.downloadModels(["embedder"])).writer).toBe("e4b");

  expect((await demoAi({ tookOver: "e2b", writer: "e2b", have: ["e2b"] }).status()).tookOver).toBe("e2b");
});
