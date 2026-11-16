/**
 * Tests run in Node, but the app's types leave Node out on purpose (the UI runs in a browser).
 * Keep the test-only surface narrow: stylesheet reads and exporting synthetic native-security fixtures.
 */
declare module "node:fs" {
  export function writeFileSync(path: string, data: string, options: { mode: number }): void;
  export function readFileSync(path: URL, encoding: "utf8"): string;
}

declare module "node:process" {
  export const env: Record<string, string | undefined>;
}
