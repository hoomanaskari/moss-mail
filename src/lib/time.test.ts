import { describe, expect, it } from "vitest";
import { dayBefore, dayBeforeDate, fullTime, listTime, remindChoices, remindTime, tomorrowMorning, weekdayOf } from "./time";

// Local times, so the tests pass in any time zone.
const now = new Date(2026, 10, 1, 15, 0).getTime(); // Sunday 1 November 2026, 3 PM

describe("list times", () => {
  it("shows the time today, the weekday this week, and the date before that", () => {
    expect(listTime(new Date(2026, 10, 1, 9, 12).getTime(), now)).toBe("9:12 AM");
    expect(listTime(new Date(2026, 10, 1, 0, 5).getTime(), now)).toBe("12:05 AM");
    expect(listTime(new Date(2026, 9, 30, 18, 0).getTime(), now)).toBe("Fri");
    expect(listTime(new Date(2026, 9, 26, 8, 0).getTime(), now)).toBe("Mon");
    expect(listTime(new Date(2026, 9, 14, 8, 0).getTime(), now)).toBe("Oct 14");
    expect(listTime(new Date(2025, 9, 14, 8, 0).getTime(), now)).toBe("Oct 14, 2025");
  });

  it("calls tomorrow tomorrow", () => {
    expect(listTime(new Date(2026, 10, 2, 8, 0).getTime(), now)).toBe("Tomorrow");
    expect(listTime(new Date(2026, 10, 9, 8, 0).getTime(), now)).toBe("Nov 9");
  });

  it("names the weekday of a snoozed email coming back later this week", () => {
    expect(listTime(new Date(2026, 10, 3, 8, 0).getTime(), now)).toBe("Tue");
    expect(listTime(new Date(2026, 10, 7, 8, 0).getTime(), now)).toBe("Sat");
  });
});

describe("full times", () => {
  it("spells out the day, with the year only when it isn't this year", () => {
    expect(fullTime(new Date(2026, 10, 1, 9, 12).getTime(), now)).toBe("Sunday, 1 November, 9:12 AM");
    expect(fullTime(new Date(2025, 0, 3, 17, 30).getTime(), now)).toBe("Friday, 3 January 2025, 5:30 PM");
  });
});

it("names the weekday", () => {
  expect(weekdayOf(now)).toBe("Sunday");
});

it("tomorrow morning is 8 AM the next day", () => {
  expect(tomorrowMorning(now)).toBe(new Date(2026, 10, 2, 8, 0).getTime());
});

const at = (...a: [number, number, number, number, number?]) => new Date(...a).getTime();

describe("remind times", () => {
  it("are the hour today, the weekday this week, and the date after that", () => {
    expect(remindTime(at(2026, 10, 1, 18, 0), now)).toBe("6 PM");
    expect(remindTime(at(2026, 10, 1, 18, 30), now)).toBe("6:30 PM");
    expect(remindTime(at(2026, 10, 2, 8, 0), now)).toBe("Mon 8 AM");
    expect(remindTime(at(2026, 10, 7, 8, 0), now)).toBe("Sat 8 AM");
    expect(remindTime(at(2026, 10, 8, 9, 15), now)).toBe("Sun 8 Nov, 9:15 AM");
    expect(remindTime(at(2027, 0, 4, 12, 0), now)).toBe("Mon 4 Jan 2027, 12 PM");
  });
});

describe("remind choices", () => {
  const ids = (n: number) => remindChoices(n).map((c) => c.id);
  const when = (n: number) => Object.fromEntries(remindChoices(n).map((c) => [c.id, c.at]));

  it("on a weekday: later today, tomorrow, this weekend and next week, all at 8 AM but the first", () => {
    const tue = at(2026, 10, 3, 13, 20); // Tuesday 1:20 PM
    expect(ids(tue)).toEqual(["today", "tomorrow", "weekend", "next-week"]);
    expect(when(tue)).toEqual({
      today: at(2026, 10, 3, 17, 0),
      tomorrow: at(2026, 10, 4, 8, 0),
      weekend: at(2026, 10, 7, 8, 0),
      "next-week": at(2026, 10, 9, 8, 0),
    });
    expect(remindChoices(tue).map((c) => c.label)).toEqual(["Later today", "Tomorrow", "This weekend", "Next week"]);
  });

  it("later today is three hours on, on the hour", () => {
    expect(when(at(2026, 10, 3, 9, 0)).today).toBe(at(2026, 10, 3, 12, 0));
    expect(when(at(2026, 10, 3, 9, 1)).today).toBe(at(2026, 10, 3, 13, 0));
  });

  it("has no later today in the evening", () => {
    expect(ids(at(2026, 10, 3, 18, 0))).toContain("today");
    expect(ids(at(2026, 10, 3, 18, 1))).not.toContain("today");
  });

  it("leaves out this weekend when it is tomorrow or already here", () => {
    expect(ids(at(2026, 10, 5, 10, 0))).toContain("weekend"); // Thursday
    expect(ids(at(2026, 10, 6, 10, 0))).not.toContain("weekend"); // Friday
    expect(ids(at(2026, 10, 7, 10, 0))).not.toContain("weekend"); // Saturday
  });

  it("next week is the coming Monday, left out on a Sunday when that is tomorrow", () => {
    expect(when(at(2026, 10, 7, 10, 0))["next-week"]).toBe(at(2026, 10, 9, 8, 0)); // Saturday
    expect(when(at(2026, 10, 9, 10, 0))["next-week"]).toBe(at(2026, 10, 16, 8, 0)); // a Monday
    expect(ids(now)).toEqual(["today", "tomorrow"]); // Sunday
  });
});

describe("the day before a deadline", () => {
  it("is 4 PM the day before the weekday it names", () => {
    expect(dayBefore("By Thursday", now)).toBe(at(2026, 10, 4, 16, 0));
    expect(dayBefore("due tuesday", now)).toBe(at(2026, 10, 2, 16, 0));
    expect(dayBefore("By Monday", now)).toBe(at(2026, 10, 1, 16, 0)); // later today
  });

  it("is nothing when that has passed or no weekday is named", () => {
    expect(dayBefore("By Monday", at(2026, 10, 1, 17, 0))).toBeUndefined(); // 4 PM today has gone
    expect(dayBefore("By Sunday", now)).toBeUndefined(); // today
    expect(dayBefore("Soon", now)).toBeUndefined();
  });

  it("counts from a real day when Moss found one", () => {
    expect(dayBeforeDate("2026-11-05", now)).toBe(at(2026, 10, 4, 16, 0));
    expect(dayBeforeDate("2026-11-02", now)).toBe(at(2026, 10, 1, 16, 0)); // later today
    expect(dayBeforeDate("2026-11-01", now)).toBeUndefined();
  });
});
