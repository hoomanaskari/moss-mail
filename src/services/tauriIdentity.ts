import { invoke } from "@tauri-apps/api/core";
import { useAppIdentity, type AppIdentity } from "./appIdentity";

/** Asks the desktop app which Moss it is. */
export async function loadAppIdentity() {
  useAppIdentity.setState(await invoke<AppIdentity>("app_identity"));
}
