import { createHash } from "node:crypto";
import { closeSync, constants, copyFileSync, fstatSync, mkdirSync, mkdtempSync, openSync, readFileSync, readSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { captureCommit, captureWorkingSource } from "./release-capture.ts";
import { snapshotSource } from "./release-source.ts";
import { validateAndPackage } from "./release-gate.ts";
import { verifyUpdate } from "./release-update.ts";
import { isVersion } from "./version.ts";

export interface ReleaseIo { env: NodeJS.ProcessEnv; log(message: string): void; run(command: string, args: string[], options: {cwd: string; env: NodeJS.ProcessEnv; capture?: boolean}): string }
const repository = "hoomanaskari/moss-mail";

/** Give each phase only the tool settings it uses. This is not an OS sandbox. */
export function releaseEnvironment(input: NodeJS.ProcessEnv, phase: "build" | "package" | "coordinator"): NodeJS.ProcessEnv {
  const extra = {
    build: [],
    package: ["MOSS_UPDATER_KEY", "MOSS_NOTARY_PROFILE", "APPLE_SIGNING_IDENTITY"],
    coordinator: ["SSH_AUTH_SOCK", "GH_TOKEN", "GITHUB_TOKEN"],
  };
  const env: NodeJS.ProcessEnv = {};
  for (const key of ["PATH", "HOME", "TMPDIR", "USER", "LOGNAME", "LANG", "LC_ALL", "DEVELOPER_DIR", "HTTP_PROXY", "HTTPS_PROXY", "NO_PROXY", ...extra[phase]]) {
    if (input[key] !== undefined) env[key] = input[key];
  }
  if (phase === "coordinator") env.GH_HOST = "github.com";
  return env;
}

function artifact(path: string, name: string): { name: string; bytes: number; sha256: string } {
  const fd = openSync(path, constants.O_RDONLY | constants.O_NOFOLLOW | constants.O_NONBLOCK);
  try {
    if (!fstatSync(fd).isFile()) throw new Error("Release artifact is not a regular file");
    const digest = createHash("sha256");
    const buffer = Buffer.alloc(1024 * 1024);
    let bytes = 0;
    for (;;) {
      const count = readSync(fd, buffer, 0, buffer.length, null);
      if (!count) break;
      digest.update(buffer.subarray(0, count)); bytes += count;
    }
    if (!bytes) throw new Error("Release artifact is empty");
    return { name, bytes, sha256: digest.digest("hex") };
  } finally { closeSync(fd); }
}

/** The only publishing entry point. All signing and notarization stays inside the gated package phase. */
export async function release(args: string[], root: string, io: ReleaseIo): Promise<{output: string}> {
  if (args.length > 1 || (args.length === 1 && args[0] !== "--local")) throw new Error("Usage: pnpm release [--local]");
  const local = args[0] === "--local";
  root = resolve(root);
  const env = releaseEnvironment(io.env, "coordinator");
  const buildEnv = releaseEnvironment(io.env, "build");
  const packageEnv = releaseEnvironment(io.env, "package");
  const run = (command: string, argv: string[], cwd = root, capture = true, commandEnv = env) => io.run(command, argv, { cwd, env: commandEnv, capture }).trim();
  if (run("uname", ["-m"]) !== "arm64") throw new Error("Build releases on an Apple silicon Mac");
  const sha = run("git", ["rev-parse", "HEAD"]);
  if (!local) {
    if (run("git", ["status", "--porcelain"])) throw new Error("There are uncommitted changes");
    if (run("git", ["branch", "--show-current"]) !== "main") throw new Error("Production releases are made from main");
    run("git", ["fetch", "-q", "origin", "main"]);
    if (sha !== run("git", ["rev-parse", "origin/main"])) throw new Error("Local main differs from origin/main");
  }
  const temporary = mkdtempSync(join(tmpdir(), "moss-release-"));
  try {
    const snapshot = join(temporary, "source");
    const source = local
      ? snapshotSource(captureWorkingSource(root), snapshot)
      : snapshotSource(captureCommit(root, sha), snapshot, { kind: "commit", sha });
    const config = JSON.parse(readFileSync(join(snapshot, "src-tauri/tauri.conf.json"), "utf8"));
    const version: unknown = config.version;
    if (typeof version !== "string" || !isVersion(version)) throw new Error("Invalid release version");
    const publicKey: unknown = config.plugins?.updater?.pubkey;
    if (typeof publicKey !== "string") throw new Error("Missing updater public key");
    const notes = join(snapshot, "docs/releases", `${version}.md`);
    if (!readFileSync(notes, "utf8").trim()) throw new Error("Release notes are empty");
    const tag = `v${version}`;
    const checkTag = () => {
      const tags = run("git", ["ls-remote", "--tags", "origin", `refs/tags/${tag}`]);
      const releases = run("gh", ["api", "--paginate", `repos/${repository}/releases`, "--jq", ".[].tag_name"]).split("\n");
      if (tags || releases.includes(tag) || run("git", ["tag", "--list", tag])) throw new Error(`${tag} already exists`);
    };
    if (!local) checkTag();
    for (const childEnv of [env, buildEnv, packageEnv]) childEnv.CARGO_TARGET_DIR = join(snapshot, "target");
    const tools: Record<string, string> = {};
    for (const [name, command, ...argv] of [
      ["node", "node", "--version"], ["pnpm", "pnpm", "--version"], ["rustc", "rustc", "--version"],
      ["minisign", "minisign", "-v"], ["cargo", "cargo", "--version"], ["swift", "swift", "--version"], ["xcode", "xcodebuild", "-version"],
    ]) {
      tools[name] = run(command, argv, snapshot, true, buildEnv);
      if (!tools[name]) throw new Error(`Could not record ${name} version`);
    }
    const provenance = { format: 1, rehearsal: local, source, tools };
    mkdirSync(join(snapshot, "target"));
    const provenanceBytes = JSON.stringify(provenance, null, 2) + "\n";
    writeFileSync(join(snapshot, "target/moss-release-source.json"), provenanceBytes);
    await validateAndPackage(snapshot, source, phase => {
      if (phase === "install") run("pnpm", ["install", "--frozen-lockfile"], snapshot, false, buildEnv);
      if (phase === "check") run("pnpm", ["check"], snapshot, false, buildEnv);
      if (phase === "package") run("sh", ["scripts/package-release.sh", local ? "--local" : "--production"], snapshot, false, packageEnv);
      if (readFileSync(join(snapshot, "target/moss-release-source.json"), "utf8") !== provenanceBytes) throw new Error("Release provenance changed during validation or packaging");
    });
    const names = [`Moss_${version}_aarch64.dmg`, `Moss_${version}_aarch64.app.tar.gz`, `Moss_${version}_aarch64.app.tar.gz.sig`, "latest.json"];
    const packaged = join(snapshot, "target/release/moss-release", version);
    const artifacts = names.map(name => artifact(join(packaged, name), name));
    const outputParent = join(root, "target/release/moss-release", local ? "local" : "production", version);
    mkdirSync(outputParent, { recursive: true });
    const output = mkdtempSync(join(outputParent, `${source.sourceHash.slice(0, 12)}-`));
    for (const name of names) copyFileSync(join(packaged, name), join(output, name), constants.COPYFILE_EXCL);
    const metadata = join(output, "release-manifest.json");
    // Validate the copied bytes, not just the package files before copying.
    if (JSON.stringify(names.map(name => artifact(join(output, name), name))) !== JSON.stringify(artifacts)) throw new Error("Release artifact changed during copy");
    verifyUpdate({ archive: join(output, names[1]), signature: readFileSync(join(output, names[2]), "utf8"),
      publicKey, version, url: `https://github.com/${repository}/releases/download/${tag}/${names[1]}`,
      manifest: JSON.parse(readFileSync(join(output, "latest.json"), "utf8")),
    }, argv => run("minisign", argv, snapshot, true, buildEnv));
    writeFileSync(metadata, JSON.stringify({ ...provenance, artifacts }, null, 2) + "\n", { flag: "wx" });
    if (!local) {
      if (source.origin.kind !== "commit" || source.origin.sha !== sha) throw new Error("Production release requires matching commit provenance");
      checkTag();
      run("gh", ["release", "create", tag, "-R", repository, "--draft", "--target", source.origin.sha, "--title", `Moss ${version}`, "--notes-file", notes, ...names.map(name => join(output, name)), metadata], snapshot, false);
    }
    io.log(`${local ? "Local rehearsal" : "Draft release"} artifacts: ${output}`);
    return { output };
  } finally {
    try { rmSync(temporary, { recursive: true, force: true }); }
    catch { io.log(`Could not remove temporary release directory: ${temporary}. Remove it after checking no release process is using it.`); }
  }
}
