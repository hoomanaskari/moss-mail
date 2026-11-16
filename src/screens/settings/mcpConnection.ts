/**
 * What to paste into each agent so it connects to Moss: a command for Claude Code, a config
 * snippet for the others. Claude Desktop only starts programs, so it runs Moss itself as a
 * bridge (`Moss mcp <url>`, with the token in its environment).
 */

export type AgentId = "claude-code" | "codex" | "claude-desktop" | "cursor" | "other";

export const agentTabs: { value: AgentId; label: string }[] = [
  { value: "claude-code", label: "Claude Code" },
  { value: "codex", label: "Codex" },
  { value: "claude-desktop", label: "Claude Desktop" },
  { value: "cursor", label: "Cursor" },
  { value: "other", label: "Other" },
];

export interface Connection {
  url: string;
  token: string;
  /** The Moss program, for Claude Desktop. */
  bridge: string;
}

export interface HowTo {
  /** Where it goes, in a few words. */
  where: string;
  /** The exact text to paste. */
  text: string;
}

const json = (v: unknown) => JSON.stringify(v, null, 2);

export function howTo(agent: AgentId, { url, token, bridge }: Connection): HowTo {
  switch (agent) {
    case "claude-code":
      return {
        where: "Run this in Terminal. It adds Moss for all your projects.",
        text: `claude mcp add --transport http --scope user moss ${url} --header "Authorization: Bearer ${token}"`,
      };
    case "codex":
      return {
        where: "Add this to ~/.codex/config.toml, then start Codex again.",
        text: `[mcp_servers.moss]\nurl = "${url}"\nhttp_headers = { "Authorization" = "Bearer ${token}" }`,
      };
    case "claude-desktop":
      return {
        where: "In Claude, open Settings, Developer, Edit Config. Add this to the file, then quit and open Claude.",
        text: json({ mcpServers: { moss: { command: bridge, args: ["mcp", url], env: { MOSS_MCP_TOKEN: token } } } }),
      };
    case "cursor":
      return {
        where: "Add this to ~/.cursor/mcp.json.",
        text: json({ mcpServers: { moss: { url, headers: { Authorization: `Bearer ${token}` } } } }),
      };
    case "other":
      return {
        where: "Any agent that connects over Streamable HTTP. Send the token as a Bearer token.",
        text: `URL: ${url}\nAuthorization: Bearer ${token}`,
      };
  }
}

/** The token as the screen shows it: enough to tell tokens apart, not enough to use. */
export function masked(token: string): string {
  return `${token.slice(0, 5)}••••••••${token.slice(-4)}`;
}

/** The snippet as shown, with the token masked; Copy takes the real one. */
export function shown(text: string, token: string): string {
  return token ? text.split(token).join(masked(token)) : text;
}

/** How long since an agent used it, in a few words. Within two minutes counts as now. */
export function lastUsedText(lastUsed: number | null | undefined, now: number): string {
  if (!lastUsed) return "No agent has connected yet";
  const minutes = Math.floor((now - lastUsed) / 60_000);
  if (minutes < 2) return "An agent is connected now";
  if (minutes < 60) return `An agent last connected ${minutes} min ago`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `An agent last connected ${hours} h ago`;
  const days = Math.floor(hours / 24);
  return `An agent last connected ${days} ${days === 1 ? "day" : "days"} ago`;
}
