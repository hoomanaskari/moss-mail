import { expect, it } from "vitest";
import { accountColorOrder } from "./types";

it("offers each of the eight account colours once, so neighbouring accounts differ", () => {
  expect(accountColorOrder).toHaveLength(8);
  expect(new Set(accountColorOrder).size).toBe(8);
});
