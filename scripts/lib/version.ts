/**
 * Moss's version lives in three files: package.json, the Tauri config and the Rust workspace.
 * `pnpm set-version 0.2.0` changes all three together, then Cargo.lock.
 */

/** What the script reads, writes, runs and says. The real ones are in scripts/set-version.ts. */
export interface Io {
  read(path: string): string;
  write(path: string, text: string): void;
  run(command: string, args: string[]): void;
  log(line: string): void;
}

/** Each file, and where in it Moss's own version is (not a dependency's). */
export const versionFiles = [
  { path: "package.json", at: /^( {2}"version": ")([^"]+)(")/m },
  { path: "src-tauri/tauri.conf.json", at: /^( {2}"version": ")([^"]+)(")/m },
  { path: "Cargo.toml", at: /(\[workspace\.package\]\nversion = ")([^"]+)(")/ },
];

/** Plain semver: 0.2.0, or 1.0.0-beta.1. */
export const isVersion = (v: string) => /^\d+\.\d+\.\d+(-[0-9A-Za-z.]+)?$/.test(v);

/** The version Moss has now, from the Tauri config. */
export const currentVersion = (io: Io) => versionFiles[1].at.exec(io.read(versionFiles[1].path))![2];

/** Sets every file to the version in `args`. 0 when done, 1 when nothing was changed. */
export function setVersion(args: string[], io: Io): number {
  const [version = ""] = args;
  if (!isVersion(version)) {
    io.log("Give the new version, like: pnpm set-version 0.2.0");
    return 1;
  }
  const was = currentVersion(io);
  const changed: [string, string][] = [];
  for (const { path, at } of versionFiles) {
    const text = io.read(path);
    if (!at.test(text)) {
      io.log(`Couldn’t find the version in ${path}.`);
      return 1;
    }
    changed.push([path, text.replace(at, `$1${version}$3`)]);
  }
  for (const [path, text] of changed) io.write(path, text);
  io.run("cargo", ["update", "--workspace", "--offline"]);
  io.log(`Moss is now ${version} (was ${was}) in ${versionFiles.map((f) => f.path).join(", ")} and Cargo.lock.`);
  return 0;
}
