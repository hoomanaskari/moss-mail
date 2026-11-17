import { describe, expect, it } from "vitest";
import { agentTabs, howTo, lastUsedText, masked, shown } from "./mcpConnection";

const c = { url: "http://127.0.0.1:47217/mcp", token: "moss_abc123def", bridge: "/Applications/Moss.app/Contents/MacOS/Moss" };

describe("what each agent gets", () => {
  it("Claude Code: one command, for every project, with the token as a header", () => {
    expect(howTo("claude-code", c).text).toBe(
      'claude mcp add --transport http --scope user moss http://127.0.0.1:47217/mcp --header "Authorization: Bearer moss_abc123def"',
    );
  });

  it("Codex: a config.toml table with a static header", () => {
    const h = howTo("codex", c);
    expect(h.where).toContain("~/.codex/config.toml");
    expect(h.text).toBe('[mcp_servers.moss]\nurl = "http://127.0.0.1:47217/mcp"\nhttp_headers = { "Authorization" = "Bearer moss_abc123def" }');
  });

  it("Claude Desktop: Moss itself as the bridge, the token in its environment", () => {
    const h = howTo("claude-desktop", c);
    expect(h.where).toContain("Edit Config");
    expect(JSON.parse(h.text)).toEqual({
      mcpServers: { moss: { command: c.bridge, args: ["mcp", c.url], env: { MOSS_MCP_TOKEN: c.token } } },
    });
  });

  it("Cursor: mcp.json with the URL and header", () => {
    const h = howTo("cursor", c);
    expect(h.where).toContain("~/.cursor/mcp.json");
    expect(JSON.parse(h.text)).toEqual({ mcpServers: { moss: { url: c.url, headers: { Authorization: "Bearer moss_abc123def" } } } });
  });

  it("Other: the URL and the header", () => {
    expect(howTo("other", c).text).toBe("URL: http://127.0.0.1:47217/mcp\nAuthorization: Bearer moss_abc123def");
  });

  it("Moss Dev goes by its own name in every agent, so it never replaces the installed Moss", () => {
    const dev = { ...c, url: "http://127.0.0.1:47218/mcp", bridge: "/Applications/Moss Dev.app/Contents/MacOS/Moss", name: "moss-dev" };
    expect(howTo("claude-code", dev).text).toContain("--scope user moss-dev http://127.0.0.1:47218/mcp");
    expect(howTo("codex", dev).text).toMatch(/^\[mcp_servers\.moss-dev\]/);
    expect(Object.keys(JSON.parse(howTo("claude-desktop", dev).text).mcpServers)).toEqual(["moss-dev"]);
    expect(Object.keys(JSON.parse(howTo("cursor", dev).text).mcpServers)).toEqual(["moss-dev"]);
  });

  it("every tab has words, and none has an em dash", () => {
    for (const t of agentTabs) {
      const h = howTo(t.value, c);
      expect(h.where.length).toBeGreaterThan(5);
      expect(h.where + h.text + t.label).not.toContain("—");
    }
  });
});

it("the token shows masked, everywhere it appears", () => {
  expect(masked("moss_abc123def")).toBe("moss_••••••••3def");
  expect(shown("a moss_abc123def b moss_abc123def", "moss_abc123def")).toBe("a moss_••••••••3def b moss_••••••••3def");
  expect(shown("no token", "")).toBe("no token");
});

it("says when an agent last connected", () => {
  const now = 1_000_000_000_000;
  expect(lastUsedText(undefined, now)).toBe("No agent has connected yet");
  expect(lastUsedText(null, now)).toBe("No agent has connected yet");
  expect(lastUsedText(now - 30_000, now)).toBe("An agent is connected now");
  expect(lastUsedText(now - 5 * 60_000, now)).toBe("An agent last connected 5 min ago");
  expect(lastUsedText(now - 3 * 3_600_000, now)).toBe("An agent last connected 3 h ago");
  expect(lastUsedText(now - 26 * 3_600_000, now)).toBe("An agent last connected 1 day ago");
  expect(lastUsedText(now - 50 * 3_600_000, now)).toBe("An agent last connected 2 days ago");
});
