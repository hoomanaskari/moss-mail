/**
 * Your own agents (Claude Code, Codex, Claude Desktop, Cursor…) connect to Moss's MCP server
 * to read mail and write drafts. In the desktop app this is `tauriAgents` (Rust: `moss-mcp`).
 * In the browser, dev scenes and tests it is the demo below.
 */

/** The connection, as Settings → AI shows it. */
export interface AgentsView {
  on: boolean;
  /** Where agents connect, while it's on. */
  url?: string | null;
  /** What they connect with, while it's on. */
  token?: string | null;
  /** When an agent last used it, in ms since the epoch; since Moss started. */
  lastUsed?: number | null;
  /** The program an agent that only runs commands (Claude Desktop) starts: Moss itself. */
  bridge: string;
}

export interface AgentsBackend {
  status(): Promise<AgentsView>;
  setOn(on: boolean): Promise<AgentsView>;
  /** A new token: the old one stops working at once. */
  newToken(): Promise<AgentsView>;
}

export const demoUrl = "http://127.0.0.1:47217/mcp";
export const demoBridge = "/Applications/Moss.app/Contents/MacOS/Moss";

/** The demo server: off unless `on`, with tokens counting up so a new one looks new. */
export function demoAgents({ on = false, lastUsed }: { on?: boolean; lastUsed?: number } = {}): AgentsBackend {
  let running = on;
  let tokens = 1;
  const token = () => `moss_${String(tokens).padStart(4, "0")}9f3c2a71e0d84b6c5a1f7e2d9c8b4a36e1f0d7c2b9a8e5f4d3c2b1a0f9e8d7`;
  const view = (): AgentsView =>
    running ? { on: true, url: demoUrl, token: token(), lastUsed, bridge: demoBridge } : { on: false, lastUsed, bridge: demoBridge };
  return {
    status: async () => view(),
    setOn: async (next) => {
      running = next;
      return view();
    },
    newToken: async () => {
      tokens += 1;
      return view();
    },
  };
}

let current: AgentsBackend = demoAgents();

export const agents = () => current;

export function setAgents(backend: AgentsBackend) {
  current = backend;
}
