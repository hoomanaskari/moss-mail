import { expect, it } from "vitest";
import { plainText } from "./markdown";

it("plain text has the Markdown taken out, for one-line places", () => {
  expect(plainText("The email is from **The Tinker Weekly** (<tinker@tinkerweekly.example>), *issue 28*: `go`.")).toBe(
    "The email is from The Tinker Weekly (tinker@tinkerweekly.example), issue 28: go.",
  );
  expect(plainText("## Heading\n\n- one\n- two<br>three")).toBe("Heading one two three");
  expect(plainText("[the site](https://x.co) &amp; more")).toBe("the site & more");
  expect(plainText("")).toBe("");
});
