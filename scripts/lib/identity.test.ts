import { describe, expect, it } from "vitest";
import { execFileSync } from "node:child_process";
import { chmodSync, mkdtempSync, readFileSync, rmSync, symlinkSync, writeFileSync, mkdirSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { assertProduction, devOverlay, development, production, tauriArgs, updateFeed } from "./identity";

const read = (path: string) => JSON.parse(readFileSync(path, "utf8"));
const shipped = read("src-tauri/tauri.conf.json");
const overlay = read("src-tauri/tauri.dev.conf.json");

describe("the two Moss apps", () => {
  it("ships the installed Moss's identity from tauri.conf.json", () => {
    expect(shipped.identifier).toBe(production.identifier);
    expect(shipped.productName).toBe(production.productName);
    expect(() => assertProduction(shipped)).not.toThrow();
  });

  it("makes Moss Dev with an overlay that changes everything keyed to the app", () => {
    expect(overlay.identifier).toBe(development.identifier);
    expect(overlay.productName).toBe(development.productName);
    expect(development.identifier).not.toBe(production.identifier);
    expect(overlay.plugins.updater.endpoints).toEqual([]);
    expect(overlay.bundle.createUpdaterArtifacts).toBe(false);
    expect(overlay.bundle.icon.every((icon: string) => icon.startsWith("icons/dev/"))).toBe(true);
  });

  it("refuses to release Moss Dev, or a Moss that would never update", () => {
    // Tauri replaces arrays and merges objects, as here.
    const merged = { ...shipped, ...overlay, app: { ...shipped.app, ...overlay.app }, plugins: { updater: { ...shipped.plugins.updater, ...overlay.plugins.updater } } };
    expect(() => assertProduction(merged)).toThrow(/identifier is app\.moss\.mail\.dev, product name is Moss Dev, the window isn't called Moss, the update feed isn't Moss's/);
    expect(() => assertProduction({})).toThrow("identifier is undefined");
    expect(() => assertProduction({ ...shipped, plugins: { updater: { endpoints: [updateFeed, "https://example.test/latest.json"] } } })).toThrow("the update feed isn't Moss's");
  });

  it("runs `tauri dev` as Moss Dev and leaves every other command alone", () => {
    expect(tauriArgs(["dev"], "/repo")).toEqual(["dev", "--config", join("/repo", "src-tauri", "tauri.dev.conf.json")]);
    expect(tauriArgs(["dev", "--release"], "/repo")).toEqual(["dev", "--config", devOverlay("/repo"), "--release"]);
    // `pnpm check` and `pnpm release` build the installed Moss through here.
    expect(tauriArgs(["build", "--no-bundle", "--", "--locked"], "/repo")).toEqual(["build", "--no-bundle", "--", "--locked"]);
    expect(tauriArgs(["signer", "sign", "-f", "key"], "/repo")).toEqual(["signer", "sign", "-f", "key"]);
    expect(tauriArgs([], "/repo")).toEqual([]);
  });

  it("builds Moss Dev, never the installed Moss, with pnpm app", () => {
    const root = mkdtempSync(join(tmpdir(), "moss-app-test-"));
    try {
      mkdirSync(join(root, "bin"));
      const trace = join(root, "trace.jsonl");
      const fake = join(root, "bin", "fake");
      writeFileSync(fake, `#!${process.execPath}\nrequire("fs").appendFileSync(process.env.TRACE, JSON.stringify({ tool: require("path").basename(process.argv[1]), args: process.argv.slice(2) }) + "\\n");\n`);
      chmodSync(fake, 0o755);
      for (const tool of ["pnpm", "security"]) symlinkSync(fake, join(root, "bin", tool));
      execFileSync("sh", ["scripts/app.sh", "--verbose"], { env: { ...process.env, PATH: `${join(root, "bin")}:${process.env.PATH}`, TRACE: trace, APPLE_SIGNING_IDENTITY: "synthetic" }, stdio: "ignore" });
      const calls = readFileSync(trace, "utf8").trim().split("\n").map((line) => JSON.parse(line));
      expect(calls).toEqual([{ tool: "pnpm", args: ["tauri", "build", "--bundles", "app", "--config", devOverlay(process.cwd()), "--verbose"] }]);
    } finally {
      rmSync(root, { recursive: true, force: true });
    }
  });
});
