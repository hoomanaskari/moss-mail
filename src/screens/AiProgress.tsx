import { AiDownloadBar } from "../design-system/molecules";
import { amount } from "../lib/bytes";
import type { AiStatus } from "../services/ai";
import { useAiSettings } from "../state/aiSettings";
import { useStore } from "../state/store";

const pausedFor = { battery: "battery below 20%", hotspot: "phone hotspot" };

/** What the top bar says while the AI downloads; nothing when it isn't. */
export function aiProgress(status?: AiStatus): { label: string; value: number; paused: boolean } | undefined {
  if (status?.download.state !== "downloading") return undefined;
  const { done, total } = status.download;
  const value = total ? done / total : 0;
  if (status.paused) return { label: `AI paused · ${pausedFor[status.paused]}`, value, paused: true };
  const lead = status.features.search ? "Search by meaning is on" : "Getting AI";
  return { label: `${lead} · ${amount(done, total)}`, value, paused: false };
}

/** The quiet download line in the top bar: no pop-ups. Opens Settings → AI. */
export function AiProgress() {
  const shown = aiProgress(useAiSettings((s) => s.status));
  const openSettings = useStore((s) => s.openSettings);
  if (!shown) return null;
  return <AiDownloadBar {...shown} onOpen={() => openSettings("ai")} />;
}

/** Under search, only when search by meaning would help and its model isn't here or coming. */
export function meaningHint(status?: AiStatus) {
  if (!status || status.features.search || status.download.state === "downloading") return undefined;
  const essentials = status.plans.find((p) => p.plan === "essentials");
  return essentials?.size.replace("About ", "");
}
