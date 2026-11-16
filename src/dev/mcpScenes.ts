import { demoAgents, setAgents } from "../services/agents";
import { pristineAgents, useAgents, type Copied } from "../state/agents";
import { useStore } from "../state/store";
import type { Scene } from "./scenes";

/** Settings → AI with "Your own agents" (at the foot of the page) on or off. */
const agentsWith = (on: boolean, extra: { copied?: Copied; lastUsed?: number } = {}) => () => {
  setAgents(demoAgents({ on, lastUsed: extra.lastUsed }));
  useAgents.setState({ ...pristineAgents, copied: extra.copied });
  // Settings may be open on AI already, loaded before this scene: load again.
  void useAgents.getState().load();
  useStore.getState().openSettings("ai");
};

export const mcpScenes: Record<string, Scene> = {
  "settings-ai-agents-off": { shows: /Connect your agents/, apply: agentsWith(false) },
  "settings-ai-agents-on": { shows: /Run this in Terminal/, apply: agentsWith(true, { lastUsed: Date.now() - 30_000 }) },
  "settings-ai-agents-copied": { shows: /^Copied$/, apply: agentsWith(true, { copied: "claude-code" }) },
};
