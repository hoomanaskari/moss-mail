import { expect, it } from "vitest";
import { execFileSync } from "node:child_process";
import { existsSync, mkdtempSync, readdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

it("keeps realistic Resend keys out of the account demo", () => {
  const demo = readFileSync("src/dev/addAccountScenes.ts", "utf8");
  expect(demo).not.toMatch(/\bre_[A-Za-z0-9]{20,}\b/);
});

// design.pen carries a private Pencil sharing ID, so the design file stays out of the public source.
it("ignores credential temporary files, mailbox process locks and the Pencil design file", () => {
  const root = mkdtempSync(join(tmpdir(), "moss-ignore-"));
  try {
    execFileSync("git", ["init", "-q"], { cwd: root });
    writeFileSync(join(root, ".gitignore"), readFileSync(".gitignore"));
    const privatePaths = ["secrets.bin", "secrets.key", "secrets.bin.tmp", "secrets.key.tmp", "mail/moss.session.lock", "mail/mail.sqlite", "mail/mail.db", "mail/mail.db-wal", "mail/mail.db-shm", "mail/mail.db-journal", "mail/mail.sqlite-journal", "mail/mail.sqlite3-journal", ".playwright-mcp/live-mail.png", ".claude/worktrees/private-mail/src/mail.ts", ".env.production", "design.pen"];
    const ignored = execFileSync("git", ["check-ignore", "--stdin"], { cwd: root, encoding: "utf8", input: [...privatePaths, "src/main.ts", ".env.example"].join("\n") + "\n" }).trim().split("\n");
    expect(ignored).toEqual(privatePaths);
  } finally { rmSync(root, { recursive: true, force: true }); }
});

it("uses reserved example domains in every model-evaluation mailbox", () => {
  // Read from the folders, not from git: a release is checked in a copy that is not a git repository.
  const folders = ["src-tauri/tests", ...readdirSync("crates").map((crate) => join("crates", crate, "tests"))].filter((folder) => existsSync(folder));
  const files = folders.flatMap((folder) => readdirSync(folder).filter((name) => /^real[^/]*\.rs$/.test(name)).map((name) => join(folder, name)));
  expect(files.length).toBeGreaterThanOrEqual(10);
  const unsafe = files.filter((path) => {
    const source = readFileSync(path, "utf8");
    const domains = [...source.matchAll(/\b[A-Za-z0-9._%+-]+@([A-Za-z0-9.-]+\.[A-Za-z]{2,})\b/g)].map((match) => match[1]);
    return domains.some((domain) => !domain.endsWith(".test") && !domain.endsWith(".example")) || source.includes("word for word");
  });
  expect(unsafe).toEqual([]);
});
