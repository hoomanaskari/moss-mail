import { verifySnapshot, type SourceManifest } from "./release-source.ts";
export type ReleasePhase = "install" | "check" | "package";

/** Returning successfully is the prerequisite for publishing any package artifacts. */
export async function validateAndPackage(root: string, manifest: SourceManifest, run: (phase: ReleasePhase) => void | Promise<void>): Promise<void> {
  verifySnapshot(root, manifest);
  for (const phase of ["install", "check", "package"] as const) {
    await run(phase);
    verifySnapshot(root, manifest);
  }
}
