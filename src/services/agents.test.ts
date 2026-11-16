import { expect, it } from "vitest";
import { agents, demoAgents, demoBridge, demoUrl, setAgents } from "./agents";

it("the demo is off until it's turned on, and shows no token while off", async () => {
  const demo = demoAgents();
  expect(await demo.status()).toEqual({ on: false, lastUsed: undefined, bridge: demoBridge });
  const on = await demo.setOn(true);
  expect(on.url).toBe(demoUrl);
  expect(on.token).toMatch(/^moss_0001/);
  expect((await demo.setOn(false)).token).toBeUndefined();
});

it("a new token is a different one", async () => {
  const demo = demoAgents({ on: true, lastUsed: 5 });
  const first = await demo.status();
  const next = await demo.newToken();
  expect(next.token).not.toBe(first.token);
  expect(next.lastUsed).toBe(5);
});

it("the app swaps in its own backend", async () => {
  const mine = demoAgents({ on: true });
  setAgents(mine);
  expect(agents()).toBe(mine);
  setAgents(demoAgents());
});
