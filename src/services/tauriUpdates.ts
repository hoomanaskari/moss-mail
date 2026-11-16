import { getVersion } from "@tauri-apps/api/app";
import { invoke } from "@tauri-apps/api/core";
import { check, type Update } from "@tauri-apps/plugin-updater";
import { noUpdate, type UpdatesBackend } from "./updates";

/** What the last check found, kept to install. */
let found: Update | null = null;

/** Tauri's updater: the newest GitHub Release's latest.json, signed with Moss's update key. */
export const tauriUpdates: UpdatesBackend = {
  version: getVersion,
  async check() {
    found = await check();
    return found && { version: found.version, notes: found.body };
  },
  async install(progress) {
    if (!found) throw new Error(noUpdate);
    let done = 0;
    let total = 0;
    await found.downloadAndInstall((e) => {
      if (e.event === "Started") total = e.data.contentLength ?? 0;
      if (e.event === "Progress") progress((done += e.data.chunkLength), total);
    });
  },
  restart: () => invoke<void>("request_restart"),
};
