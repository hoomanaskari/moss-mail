import { execFileSync } from "node:child_process";
import { closeSync, constants, fstatSync, lstatSync, openSync, readFileSync } from "node:fs";
import { join } from "node:path";
import type { SourceEntry } from "./release-source.ts";

function git(root: string, args: string[]): Buffer {
  const env = Object.fromEntries(Object.entries(process.env).filter(([key]) => !key.startsWith("GIT_")));
  return execFileSync("git", args, { cwd: root, env, maxBuffer: 128 * 1024 * 1024 });
}

/** Git blobs are addressed by object identity, never by a moving branch name. */
export function captureCommit(root: string, sha: string): SourceEntry[] {
  if (!/^[a-f0-9]{40}$/.test(sha)) throw new Error("Expected a full commit identifier");
  if (git(root, ["cat-file", "-t", sha]).toString("utf8").trim() !== "commit") throw new Error("Expected a commit object");
  const records = git(root, ["ls-tree", "-rz", "--full-tree", sha]).toString("utf8").split("\0").filter(Boolean);
  return records.map(record => {
    const tab = record.indexOf("\t");
    const [mode, type, object] = record.slice(0, tab).split(" ");
    if (type !== "blob" || (mode !== "100644" && mode !== "100755")) throw new Error("Release source must contain only regular files");
    return { path: record.slice(tab + 1), bytes: git(root, ["cat-file", "blob", object]), executable: mode === "100755" };
  });
}

/** Local rehearsal includes dirty source, with its own manifest rather than a commit claim. */
export function captureWorkingSource(root: string): SourceEntry[] {
  const paths = [...new Set(git(root, ["ls-files", "--cached", "--others", "--exclude-standard", "-z"]).toString("utf8").split("\0").filter(Boolean))].sort();
  const entries: SourceEntry[] = [];
  for (const path of paths) {
    let parent = root;
    const parts = path.split("/");
    try {
      for (const part of parts.slice(0, -1)) {
        parent = join(parent, part);
        if (!lstatSync(parent).isDirectory()) throw new Error("Release source must contain only regular files and directories");
      }
      const full = join(root, path);
      if (!lstatSync(full).isFile()) throw new Error("Release source must contain only regular files");
      const fd = openSync(full, constants.O_RDONLY | constants.O_NOFOLLOW | constants.O_NONBLOCK);
      try {
        const stat = fstatSync(fd);
        if (!stat.isFile()) throw new Error("Release source must contain only regular files");
        entries.push({ path, bytes: readFileSync(fd), executable: (stat.mode & 0o111) !== 0 });
      } finally { closeSync(fd); }
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code !== "ENOENT") throw error;
      // Tracked files deleted in the working copy are intentionally absent locally.
    }
  }
  return entries;
}
