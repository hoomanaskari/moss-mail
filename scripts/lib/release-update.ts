import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

export interface UpdateArtifact { archive: string; signature: string; publicKey: string; version: string; url: string; manifest: unknown }

function decode(value: string): string {
  const encoded = value.trim();
  const bytes = Buffer.from(encoded, "base64");
  if (!encoded || bytes.toString("base64") !== encoded) throw new Error("Invalid updater base64 encoding");
  return new TextDecoder("utf-8", { fatal: true }).decode(bytes);
}

/** Delegate cryptography to minisign, then check the authenticated version and release manifest. */
export function verifyUpdate(update: UpdateArtifact, run: (args: string[]) => string): void {
  const latest = update.manifest as {version?: unknown; platforms?: {"darwin-aarch64"?: {url?: unknown; signature?: unknown}}} | null;
  const platform = latest?.platforms?.["darwin-aarch64"];
  if (latest?.version !== update.version || platform?.url !== update.url || platform?.signature !== update.signature.trim()) throw new Error("Inconsistent update manifest");
  const publicKey = decode(update.publicKey);
  const signature = decode(update.signature);
  const temporary = mkdtempSync(join(tmpdir(), "moss-update-verify-"));
  try {
    const pub = join(temporary, "public-key");
    const sig = join(temporary, "signature");
    writeFileSync(pub, publicKey, { flag: "wx", mode: 0o600 });
    writeFileSync(sig, signature, { flag: "wx", mode: 0o600 });
    run(["-V", "-q", "-p", pub, "-x", sig, "-m", update.archive]);
    // minisign has now authenticated the trusted comment as well as the archive.
    const comment = signature.split(/\r?\n/)[2];
    const versions = comment.slice("trusted comment: ".length).split("\t").filter(field => field.startsWith("version:"));
    if (versions.length !== 1 || versions[0] !== `version:${update.version}`) throw new Error("Missing or mismatched signed version");
  } finally { rmSync(temporary, { recursive: true, force: true }); }
}
