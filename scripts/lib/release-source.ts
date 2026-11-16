import { createHash } from "node:crypto";
import { chmodSync, lstatSync, mkdirSync, readFileSync, readdirSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";

export interface SourceEntry { path: string; bytes: Uint8Array; executable?: boolean }
export type SourceOrigin = { kind: "working" } | { kind: "commit"; sha: string };
export interface SourceManifest { format: 1; origin: SourceOrigin; sourceHash: string; files: { path: string; bytes: number; sha256: string; executable: boolean }[] }
const outputs = ["node_modules", "target", "dist", "coverage", "apple/.build", "src-tauri/gen", "src-tauri/binaries"];
const isOutput = (path: string) => outputs.some(root => path === root || path.startsWith(`${root}/`));
const hash = (bytes: Uint8Array | string) => createHash("sha256").update(bytes).digest("hex");

function checkPath(path: string): void {
  const parts = path.split("/");
  if (!path || /[\\:\x00-\x1f\x7f]/.test(path) || parts.some(part => !part || part === "." || part === "..")) throw new Error("Unsafe source path");
  if (isOutput(path.toLowerCase()) || parts.map(part => part.toLowerCase()).some(part => part === ".git" || part === ".aws" || (part.startsWith(".env") && part !== ".env.example") || /^secrets\./.test(part) || /\.(?:key|p12|pfx|sqlite|sqlite3|db)(?:-wal|-shm|\.tmp)?$/.test(part) || part.endsWith(".session.lock"))) throw new Error(`Private or generated source input: ${path}`);
}

function manifestFor(entries: SourceEntry[], origin: SourceOrigin): SourceManifest {
  const files = entries.map(entry => ({ path: entry.path, bytes: entry.bytes.byteLength, sha256: hash(entry.bytes), executable: entry.executable === true })).sort((a, b) => a.path < b.path ? -1 : 1);
  return { format: 1, origin, sourceHash: hash(JSON.stringify({ origin, files })), files };
}

/** The destination must be new. Validate every entry before writing any bytes. */
export function snapshotSource(entries: SourceEntry[], destination: string, origin: SourceOrigin = { kind: "working" }): SourceManifest {
  if (origin.kind === "commit" && !/^[a-f0-9]{40}$/.test(origin.sha)) throw new Error("Expected a full commit identifier");
  const paths = new Set<string>();
  for (const entry of entries) {
    checkPath(entry.path);
    if (paths.has(entry.path)) throw new Error("Duplicate source path");
    paths.add(entry.path);
  }
  for (const path of paths) {
    const parts = path.split("/");
    while (parts.pop() && parts.length) if (paths.has(parts.join("/"))) throw new Error("Conflicting source paths");
  }
  mkdirSync(destination, { mode: 0o700 });
  for (const entry of entries) {
    const path = join(destination, entry.path);
    mkdirSync(dirname(path), { recursive: true });
    writeFileSync(path, entry.bytes, { flag: "wx", mode: 0o600 });
    chmodSync(path, entry.executable ? 0o755 : 0o644);
  }
  return manifestFor(entries, origin);
}

/** Build products may appear, but all other files must match the validated source. */
export function verifySnapshot(root: string, manifest: SourceManifest): void {
  if (!lstatSync(root).isDirectory()) throw new Error("Invalid snapshot root");
  const entries: SourceEntry[] = [];
  function walk(relative: string): void {
    for (const name of readdirSync(join(root, relative))) {
      const path = relative ? `${relative}/${name}` : name;
      if (isOutput(path)) {
        if (!lstatSync(join(root, path)).isDirectory()) throw new Error("Invalid build output directory");
        continue;
      }
      checkPath(path);
      const stat = lstatSync(join(root, path));
      if (stat.isDirectory()) walk(path);
      else if (stat.isFile()) entries.push({ path, bytes: readFileSync(join(root, path)), executable: (stat.mode & 0o111) !== 0 });
      else throw new Error(`Non-regular source entry: ${path}`);
    }
  }
  walk("");
  if (JSON.stringify(manifestFor(entries, manifest.origin)) !== JSON.stringify(manifest)) throw new Error("Release source changed after capture");
}
