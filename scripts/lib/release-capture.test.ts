import { afterEach, describe, expect, it, vi } from "vitest";
import * as fs from "node:fs";
import { execFileSync } from "node:child_process";
import { chmodSync, mkdirSync, mkdtempSync, rmSync, symlinkSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { captureCommit, captureWorkingSource } from "./release-capture";
vi.mock("node:fs", async importOriginal => {
  const actual = await importOriginal<typeof import("node:fs")>();
  return { ...actual, fstatSync: vi.fn(actual.fstatSync) };
});
const folders: string[] = [];
afterEach(() => { vi.unstubAllEnvs(); vi.mocked(fs.fstatSync).mockReset(); for (const dir of folders.splice(0)) rmSync(dir, { recursive: true, force: true }); });
function repo() {
  const root = mkdtempSync(join(tmpdir(), "moss-capture-")); folders.push(root);
  const git = (...args: string[]) => execFileSync("git", args, { cwd: root, encoding: "utf8" }).trim();
  git("init", "-q"); git("config", "user.name", "Test"); git("config", "user.email", "test@example.invalid");
  writeFileSync(join(root, "main.ts"), "original");
  writeFileSync(join(root, "build.sh"), "#!/bin/sh\n"); chmodSync(join(root, "build.sh"), 0o755);
  git("add", "."); git("commit", "-qm", "fixture");
  return { root, git, sha: git("rev-parse", "HEAD") };
}
describe("release source capture", () => {
  it("ignores inherited Git repository overrides", () => {
    const { root, sha } = repo();
    vi.stubEnv("GIT_DIR", "/nonexistent-release-git-override");
    expect(captureCommit(root, sha).map(e => e.path)).toEqual(["build.sh", "main.ts"]);
    expect(captureWorkingSource(root).map(e => e.path)).toEqual(["build.sh", "main.ts"]);
  });
  it("captures nested regular files and executable local files", () => {
    const { root } = repo(); mkdirSync(join(root, "src")); writeFileSync(join(root, "src/a"), "nested");
    const entries = captureWorkingSource(root);
    expect(entries.find(e => e.path === "src/a")?.bytes.toString()).toBe("nested");
    expect(entries.find(e => e.path === "build.sh")?.executable).toBe(true);
  });
  it("rejects a non-regular file discovered after opening", () => {
    const { root } = repo();
    vi.mocked(fs.fstatSync).mockReturnValueOnce({ isFile: () => false } as fs.Stats);
    expect(() => captureWorkingSource(root)).toThrow("regular files");
  });
  it("rejects Git submodules instead of omitting their source", () => {
    const { root, git, sha } = repo();
    git("update-index", "--add", "--cacheinfo", `160000,${sha},submodule`); git("commit", "-qm", "submodule");
    expect(() => captureCommit(root, git("rev-parse", "HEAD"))).toThrow("regular files");
  });
  it("reads the exact commit despite dirty source and a moved HEAD", () => {
    const { root, git, sha } = repo();
    writeFileSync(join(root, "main.ts"), "later"); git("add", "."); git("commit", "-qm", "later");
    writeFileSync(join(root, "main.ts"), "dirty");
    const entries = captureCommit(root, sha);
    expect(entries.map(e => [e.path, Buffer.from(e.bytes).toString(), e.executable])).toEqual([["build.sh", "#!/bin/sh\n", true], ["main.ts", "original", false]]);
  });
  it("refuses a tree object as a release commit", () => {
    const { root, git } = repo();
    expect(() => captureCommit(root, git("rev-parse", "HEAD^{tree}"))).toThrow("commit object");
  });
  it("requires a full immutable commit identifier", () => {
    const { root } = repo();
    for (const value of ["HEAD", "main", "--help", "bad"]) expect(() => captureCommit(root, value)).toThrow("commit identifier");
  });
  it("rejects committed symlinks", () => {
    const { root, git } = repo(); symlinkSync("main.ts", join(root, "link")); git("add", "."); git("commit", "-qm", "link");
    expect(() => captureCommit(root, git("rev-parse", "HEAD"))).toThrow("regular files");
  });
  it("captures local edits, additions and deletions, excluding ignored files", () => {
    const { root } = repo(); writeFileSync(join(root, ".gitignore"), "ignored\n");
    writeFileSync(join(root, "main.ts"), "dirty"); rmSync(join(root, "build.sh"));
    writeFileSync(join(root, "new.ts"), "new"); writeFileSync(join(root, "ignored"), "private");
    const entries = captureWorkingSource(root);
    expect(entries.map(e => e.path)).toEqual([".gitignore", "main.ts", "new.ts"]);
    expect(Buffer.from(entries.find(e => e.path === "main.ts")!.bytes).toString()).toBe("dirty");
  });
  it("rejects a local symlink and a tracked directory replaced by a symlink", () => {
    const { root, git } = repo(); symlinkSync("main.ts", join(root, "link"));
    expect(() => captureWorkingSource(root)).toThrow("regular files"); rmSync(join(root, "link"));
    mkdirSync(join(root, "src")); writeFileSync(join(root, "src/a"), "source"); git("add", "."); git("commit", "-qm", "nested");
    rmSync(join(root, "src"), { recursive: true }); symlinkSync(root, join(root, "src"));
    writeFileSync(join(root, ".gitignore"), "/src\n");
    expect(() => captureWorkingSource(root)).toThrow("regular files");
  });
});
