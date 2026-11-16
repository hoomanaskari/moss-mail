import { expect, it } from "vitest";
import { fileSize, megabytes, sendLimit, tooBig } from "./attachments";

const MB = 1024 * 1024;

// Same cases as crates/moss-engine/src/attach.rs.
it("each mail service takes its own amount", () => {
  expect(sendLimit("gmail") / MB).toBe(25);
  expect(sendLimit("yahoo") / MB).toBe(25);
  expect(sendLimit("outlook") / MB).toBe(20);
  expect(sendLimit("icloud") / MB).toBe(20);
  expect(sendLimit("fastmail") / MB).toBe(50);
  expect(sendLimit("resend") / MB).toBe(30);
  expect(sendLimit("imap") / MB).toBe(25);
  // A demo account has no connection: the usual 25 MB.
  expect(sendLimit(undefined) / MB).toBe(25);
});

it("too much says how much and what fits", () => {
  expect(tooBig(25 * MB, "gmail")).toBeUndefined();
  expect(tooBig(25 * MB + 1, "gmail")).toBe("These files are 25 MB. Gmail sends up to 25 MB.");
  expect(tooBig(32_715_571, "icloud")).toBe("These files are 31.2 MB. iCloud sends up to 20 MB.");
  expect(tooBig(26 * MB, "imap")).toBe("These files are 26 MB. Your mail server sends up to 25 MB.");
  expect(tooBig(26 * MB, undefined)).toBe("These files are 26 MB. Your mail server sends up to 25 MB.");
  for (const [p, name] of [["outlook", "Outlook"], ["yahoo", "Yahoo"], ["fastmail", "Fastmail"], ["resend", "Resend"]] as const)
    expect(tooBig(51 * MB, p)).toContain(name);
});

it("megabytes drop a trailing .0", () => {
  expect(megabytes(25 * MB)).toBe("25 MB");
  expect(megabytes(1.26 * MB)).toBe("1.3 MB");
});

// Same cases as `format_size` in crates/moss-core/src/mail.rs, which sizes received files.
it("a file's size reads like a received one's", () => {
  expect(fileSize(0)).toBe("0 B");
  expect(fileSize(1023)).toBe("1023 B");
  expect(fileSize(1024)).toBe("1 KB");
  expect(fileSize(245_760)).toBe("240 KB");
  expect(fileSize(1_048_575)).toBe("1024 KB");
  expect(fileSize(1_258_291)).toBe("1.2 MB");
  expect(fileSize(3 * 1024 * 1024 * 1024)).toBe("3.0 GB");
});
