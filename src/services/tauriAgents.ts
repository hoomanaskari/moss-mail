/** The MCP server in the desktop app: each call is one Tauri command (`src-tauri/src/mcp.rs`). */
import { invoke } from "@tauri-apps/api/core";
import type { AgentsBackend, AgentsView } from "./agents";

export const tauriAgents: AgentsBackend = {
  status: () => invoke<AgentsView>("mcp_status"),
  setOn: (on) => invoke<AgentsView>("mcp_set_on", { on }),
  newToken: () => invoke<AgentsView>("mcp_new_token"),
};
