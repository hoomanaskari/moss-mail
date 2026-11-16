import { afterEach, describe, expect, it } from "vitest";
import { chmodSync, existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, symlinkSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { snapshotSource, verifySnapshot, type SourceEntry } from "./release-source";

const folders: string[] = [];
afterEach(() => { for (const dir of folders.splice(0)) rmSync(dir, { recursive: true, force: true }); });
function fixture() {
  const dir = mkdtempSync(join(tmpdir(), "moss-source-test-"));
  folders.push(dir);
  return { dir, root: join(dir, "source") };
}
const entry = (path: string, text = "source", executable = false): SourceEntry => ({ path, bytes: Buffer.from(text), executable });

describe("release source snapshots", () => {
  it("binds source provenance into its identity", () => {
    const entries = [entry("main.ts")];
    const working = snapshotSource(entries, fixture().root);
    const committed = snapshotSource(entries, fixture().root, { kind: "commit", sha: "a".repeat(40) });
    expect(working.origin).toEqual({ kind: "working" });
    expect(committed.origin).toEqual({ kind: "commit", sha: "a".repeat(40) });
    expect(working.sourceHash).not.toBe(committed.sourceHash);
    const { root } = fixture();
    const manifest = snapshotSource(entries, root, { kind: "commit", sha: "a".repeat(40) });
    manifest.origin = { kind: "working" };
    expect(() => verifySnapshot(root, manifest)).toThrow("source changed");
  });
  it("refuses malformed commit provenance", () => {
    expect(() => snapshotSource([entry("main.ts")], fixture().root, { kind: "commit", sha: "HEAD" })).toThrow("commit identifier");
  });
  it("captures exact bytes and executable modes with a stable source identity", () => {
    const { root } = fixture();
    const manifest = snapshotSource([entry("src/main.ts", "export const value = 1;"), entry("scripts/build.sh", "#!/bin/sh\n", true)], root);
    expect(readFileSync(join(root, "src/main.ts"), "utf8")).toBe("export const value = 1;");
    expect(manifest.files.map(file => file.path)).toEqual(["scripts/build.sh", "src/main.ts"]);
    expect(manifest.sourceHash).toMatch(/^[a-f0-9]{64}$/);
    expect(manifest.files[0]).toMatchObject({ executable: true, bytes: 10 });
    expect(() => verifySnapshot(root, manifest)).not.toThrow();
    const other = fixture();
    expect(snapshotSource([entry("scripts/build.sh", "#!/bin/sh\n", true), entry("src/main.ts", "export const value = 1;")], other.root).sourceHash).toBe(manifest.sourceHash);
  });

  it.each(["../escape", "/absolute", "a/../escape", "a//file", "a/./file", "C:\\file", "a\\b", "bad\nname"])("rejects unsafe source paths: %j", path => {
    const { dir, root } = fixture();
    expect(() => snapshotSource([entry(path)], root)).toThrow();
    expect(existsSync(join(dir, "escape"))).toBe(false);
  });

  it.each([".git/config", ".aws/credentials", ".ENV", "Secrets.json", "certificate.P12", ".env", "src/.env.production", "secrets.key", "secrets.bin.tmp", "secrets.key.tmp", "mail.sqlite", "moss.session.lock", "node_modules/pkg/index.js", "target/generated.rs"])("refuses credential or generated inputs: %s", path => {
    expect(() => snapshotSource([entry(path)], fixture().root)).toThrow();
  });

  it("allows a public environment example and rejects duplicate or conflicting paths", () => {
    expect(() => snapshotSource([entry(".env.example", "EXAMPLE=")], fixture().root)).not.toThrow();
    expect(() => snapshotSource([entry("src/a"), entry("src/a")], fixture().root)).toThrow();
    expect(() => snapshotSource([entry("src/a"), entry("src/a/b")], fixture().root)).toThrow();
  });

  it("does not overwrite an existing destination or follow its symlink", () => {
    const { dir, root } = fixture();
    mkdirSync(root);
    writeFileSync(join(root, "kept"), "keep me");
    expect(() => snapshotSource([entry("new")], root)).toThrow();
    expect(readFileSync(join(root, "kept"), "utf8")).toBe("keep me");
    const link = join(dir, "linked");
    symlinkSync(root, link);
    expect(() => snapshotSource([entry("new")], link)).toThrow();
  });

  it.each(["edit", "add", "remove", "mode", "symlink"])("detects a source %s after validation", change => {
    const { dir, root } = fixture();
    const manifest = snapshotSource([entry("src/main.ts")], root);
    const path = join(root, "src/main.ts");
    if (change === "edit") writeFileSync(path, "changed");
    if (change === "add") writeFileSync(join(root, "src/injected.ts"), "added");
    if (change === "remove") rmSync(path);
    if (change === "mode") chmodSync(path, 0o755);
    if (change === "symlink") {
      const outside = join(dir, "outside");
      writeFileSync(outside, "source");
      rmSync(path);
      symlinkSync(outside, path);
    }
    expect(() => verifySnapshot(root, manifest)).toThrow();
  });

  it("rejects symlinked output directories", () => {
    const { dir, root } = fixture();
    const manifest = snapshotSource([entry("main.ts")], root);
    const outside = join(dir, "outside"); mkdirSync(outside);
    symlinkSync(outside, join(root, "target"));
    expect(() => verifySnapshot(root, manifest)).toThrow("output directory");
  });
  it("rejects a replaced snapshot root", () => {
    const { dir, root } = fixture();
    const manifest = snapshotSource([entry("main.ts")], root);
    const link = join(dir, "root-link");
    symlinkSync(root, link);
    expect(() => verifySnapshot(link, manifest)).toThrow("Invalid snapshot root");
  });

  it("allows build outputs but does not let a generated environment file change the build", () => {
    const { root } = fixture();
    const manifest = snapshotSource([entry("src/main.ts")], root);
    for (const folder of ["node_modules", "target", "dist", "coverage", "apple/.build", "src-tauri/gen", "src-tauri/binaries"]) {
      mkdirSync(join(root, folder), { recursive: true });
      writeFileSync(join(root, folder, "output"), "generated");
    }
    expect(() => verifySnapshot(root, manifest)).not.toThrow();
    writeFileSync(join(root, ".env.production"), "UNEXPECTED=1");
    expect(() => verifySnapshot(root, manifest)).toThrow();
  });
});
