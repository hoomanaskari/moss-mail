import { describe, expect, it } from "vitest";
import { needsICloudLogin } from "./providers";

describe("needsICloudLogin", () => {
  // The same cases as `MailService::for_address` and the custom-domain tests in moss-accounts.
  it.each([
    ["info@fernway.co", true],
    [" info@fernway.co ", true],
    ["maya@icloud.com", false],
    ["maya@me.com", false],
    ["maya@mac.com", false],
    ["MAYA@ICLOUD.COM", false],
    // Not an address yet: nothing to ask for.
    ["", false],
    ["info@", false],
    ["info@fernway", false],
  ])("%j → %s", (email, needed) => {
    expect(needsICloudLogin(email)).toBe(needed);
  });
});
