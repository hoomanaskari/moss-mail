import { expect, it } from "vitest";
import { amount, size } from "./bytes";

it("sizes read like people say them", () => {
  expect(size(38_000_000)).toBe("38 MB");
  expect(size(6_410_000_000)).toBe("6.4 GB");
  expect(size(1_000_000_000)).toBe("1 GB");
});

it("an amount names the unit once when both share it", () => {
  expect(amount(2_400_000_000, 6_400_000_000)).toBe("2.4 of 6.4 GB");
  expect(amount(450_000_000, 6_400_000_000)).toBe("450 MB of 6.4 GB");
});
