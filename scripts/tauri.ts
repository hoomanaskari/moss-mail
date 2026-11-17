// `pnpm tauri <args>`: Tauri's CLI, with `tauri dev` running Moss Dev (see lib/identity.ts).
import { spawnSync } from "node:child_process";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { tauriArgs } from "./lib/identity.ts";

const root = fileURLToPath(new URL("..", import.meta.url));
const cli = join(root, "node_modules", "@tauri-apps", "cli", "tauri.js");
const done = spawnSync(process.execPath, [cli, ...tauriArgs(process.argv.slice(2), root)], { stdio: "inherit" });
process.exitCode = done.status ?? 1;
