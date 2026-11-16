import { describe, expect, it } from "vitest";
import { thread } from "../test/fixtures";
import { doneGroups } from "./done";

const now = new Date(2026, 10, 2, 10, 30).getTime();
const at = (day: number, hour = 9) => new Date(2026, 10, day, hour).getTime();

describe("doneGroups", () => {
  it("groups by the day each email was done, keeping the order it is given", () => {
    const groups = doneGroups(
      [
        thread("a", { doneAt: at(2, 10) }),
        thread("b", { doneAt: at(2, 8) }),
        thread("c", { doneAt: at(1) }),
        thread("d", { doneAt: new Date(2026, 9, 29).getTime() }),
        thread("e", { doneAt: new Date(2026, 9, 20).getTime() }),
      ],
      now,
    );
    expect(groups.map((g) => [g.title, g.threads.map((t) => t.id)])).toEqual([
      ["Today", ["a", "b"]],
      ["Yesterday", ["c"]],
      ["Earlier this week", ["d"]],
      ["Older", ["e"]],
    ]);
  });

  it("puts emails with no done time under Older, and leaves out empty groups", () => {
    expect(doneGroups([thread("a")], now)).toEqual([{ title: "Older", threads: [thread("a")] }]);
    expect(doneGroups([], now)).toEqual([]);
  });

  it("counts six days back as this week and seven as older", () => {
    const groups = doneGroups([thread("six", { doneAt: now - 6 * 86_400_000 }), thread("seven", { doneAt: now - 7 * 86_400_000 })], now);
    expect(groups.map((g) => g.title)).toEqual(["Earlier this week", "Older"]);
  });

  it("can group by another time, such as when blocked mail came", () => {
    const groups = doneGroups([thread("a", { lastMessageAt: at(2), doneAt: at(1) })], now, (t) => t.lastMessageAt);
    expect(groups.map((g) => g.title)).toEqual(["Today"]);
  });
});
