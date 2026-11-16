import { create } from "zustand";
import { agents, type AgentsView } from "../services/agents";
import { errorText } from "../services/errorText";
import type { AgentId } from "../screens/settings/mcpConnection";

/** What was copied last: an agent's snippet, or the URL or token on Other. */
export type Copied = AgentId | "url" | "token";

interface AgentsState {
  view?: AgentsView;
  /** The agent whose connection shows. */
  agent: AgentId;
  copied?: Copied;
  /** "New token" was pressed once: press again to make it. */
  asking: boolean;
  problem?: string;
  load(): Promise<void>;
  setOn(on: boolean): Promise<void>;
  newToken(): Promise<void>;
  choose(agent: AgentId): void;
  copy(what: Copied, text: string): Promise<void>;
}

/** How long "Copied" stays. */
export const COPIED_FOR = 2000;

export const pristineAgents = { view: undefined, agent: "claude-code" as AgentId, copied: undefined, asking: false, problem: undefined };

/** Settings → AI → Your own agents: the MCP server, on or off, and how to connect to it. */
export const useAgents = create<AgentsState>()((set, get) => {
  const run = async (call: () => Promise<AgentsView>) => {
    try {
      set({ view: await call(), problem: undefined });
    } catch (e) {
      set({ problem: errorText(e) });
    }
  };
  return {
    ...pristineAgents,
    load: () => run(() => agents().status()),
    setOn: (on) => {
      set({ asking: false });
      return run(() => agents().setOn(on));
    },
    newToken: async () => {
      if (!get().asking) return set({ asking: true });
      set({ asking: false });
      await run(() => agents().newToken());
    },
    choose: (agent) => set({ agent, copied: undefined }),
    copy: async (what, text) => {
      try {
        await navigator.clipboard.writeText(text);
      } catch {
        return set({ problem: "Couldn’t copy. Select the text and copy it yourself." });
      }
      set({ copied: what, problem: undefined });
      setTimeout(() => get().copied === what && set({ copied: undefined }), COPIED_FOR);
    },
  };
});
