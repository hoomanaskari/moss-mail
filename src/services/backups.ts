export type BackupProgress =
  | { phase: "preparing" | "checking" | "hashing" | "publishing" }
  | { phase: "copying"; copiedPages: number; totalPages: number };
export type BackupStatus =
  | { state: "idle" }
  | { state: "running"; id: number; progress: BackupProgress; cancelling: boolean }
  | { state: "complete"; id: number; bytes: number; sha256: string }
  | { state: "cancelled"; id: number }
  | { state: "failed"; id: number; message: string };

/** Refuse unknown or malformed native state instead of enabling another job. */
export function isBackupStatus(value: unknown): value is BackupStatus {
  if (!value || typeof value !== "object") return false;
  const status = value as Record<string, unknown>;
  if (status.state === "idle") return true;
  const count = (n: unknown): n is number => typeof n === "number" && Number.isSafeInteger(n) && n >= 0;
  if (!count(status.id) || status.id === 0 || status.id > 0xffff_ffff) return false;
  if (status.state === "cancelled") return true;
  if (status.state === "failed") return typeof status.message === "string";
  if (status.state === "complete") return count(status.bytes) && typeof status.sha256 === "string" && /^[a-f0-9]{64}$/.test(status.sha256);
  if (status.state !== "running" || typeof status.cancelling !== "boolean" || !status.progress || typeof status.progress !== "object") return false;
  const progress = status.progress as Record<string, unknown>;
  if (progress.phase === "copying") return count(progress.copiedPages) && count(progress.totalPages) && progress.copiedPages <= progress.totalPages && progress.totalPages <= 0xffff_ffff;
  return ["preparing", "checking", "hashing", "publishing"].includes(progress.phase as string);
}
