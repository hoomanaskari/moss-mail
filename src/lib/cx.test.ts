import { expect, it } from "vitest";
import { cx } from "./cx";

it("joins class names and skips empty ones", () => {
  expect(cx("a", false, null, undefined, "", "b")).toBe("a b");
});
