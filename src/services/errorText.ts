/** What the engine's errors say, as a sentence for the screen. */
export function errorText(error: unknown): string {
  if (typeof error === "string") return error;
  if (error && typeof error === "object" && "message" in error && typeof error.message === "string") return error.message;
  return "Something went wrong. Try again.";
}
