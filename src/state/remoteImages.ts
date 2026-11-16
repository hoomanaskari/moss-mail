import { create } from "zustand";

export type RemoteImagePolicy = "familiar" | "ask";
const key = "moss.remoteImages";

/** Missing means the existing default. Unreadable privacy choices fail closed. */
export function readRemoteImagePolicy(): { policy: RemoteImagePolicy; saved: boolean } {
  try {
    const policy = localStorage.getItem(key);
    if (policy === null) return { policy: "familiar", saved: true };
    if (policy === "familiar" || policy === "ask") return { policy, saved: true };
  } catch { /* Keep remote content hidden when storage cannot be read. */ }
  return { policy: "ask", saved: false };
}

export const useRemoteImages = create<{ policy: RemoteImagePolicy; saved: boolean; setPolicy: (policy: RemoteImagePolicy) => void }>((set) => ({
  ...readRemoteImagePolicy(),
  setPolicy: (policy) => {
    let saved = true;
    try { localStorage.setItem(key, policy); } catch { saved = false; }
    set({ policy, saved });
  },
}));
