import { act, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { demoAgents, demoBridge, demoUrl, setAgents, type AgentsBackend } from "../../services/agents";
import { pristineAgents, useAgents } from "../../state/agents";
import { installedMoss, mossDev, useAppIdentity } from "../../services/appIdentity";
import { McpSection } from "./McpSection";

const NOW = 1_800_000_000_000;
let writeText: ReturnType<typeof vi.fn>;

async function show(backend: AgentsBackend = demoAgents()) {
  setAgents(backend);
  render(<McpSection now={() => NOW} />);
  await act(async () => {});
}

/** userEvent brings its own clipboard: ours goes in after it, to see what was copied. */
function clipboard() {
  Object.defineProperty(navigator, "clipboard", { value: { writeText }, configurable: true });
}

function setupUser(options?: Parameters<typeof userEvent.setup>[0]) {
  const user = userEvent.setup(options);
  clipboard();
  return user;
}

beforeEach(() => {
  useAgents.setState(pristineAgents);
  writeText = vi.fn(async () => {});
  clipboard();
});

afterEach(() => setAgents(demoAgents()));

it("is off at first, with the privacy line and no connection to copy", async () => {
  await show();
  expect(screen.getByRole("switch", { name: "Connect your agents" })).toHaveAttribute("aria-checked", "false");
  expect(screen.getByText("Agents you connect can read, manage and send your email. A cloud agent sends what it reads to its company.")).toBeInTheDocument();
  // People may look for "MCP" rather than "agents": both are named.
  expect(screen.getByRole("heading", { name: "Your own agents (MCP)" })).toBeInTheDocument();
  expect(screen.getByText("Let Claude Code, Codex, Cursor and others manage your mail through MCP: sort it, answer it, send it.")).toBeInTheDocument();
  expect(screen.queryByRole("radiogroup", { name: "Agent" })).not.toBeInTheDocument();
  expect(screen.queryByRole("button", { name: /Copy/ })).not.toBeInTheDocument();
});

it("shows nothing until it knows whether it's on", () => {
  setAgents({ ...demoAgents(), status: () => new Promise(() => {}) });
  render(<McpSection />);
  expect(screen.queryByLabelText("Your own agents")).not.toBeInTheDocument();
});

it("turned on, shows Claude Code's command with the token masked, and copies the real one", async () => {
  const user = setupUser();
  await show();
  await user.click(screen.getByRole("switch", { name: "Connect your agents" }));
  const shownText = screen.getByLabelText("Connection for Claude Code").textContent!;
  expect(shownText).toContain(`claude mcp add --transport http --scope user moss ${demoUrl}`);
  expect(shownText).toContain("moss_••••••••");
  expect(shownText).not.toContain(useAgents.getState().view!.token!);
  await user.click(screen.getByRole("button", { name: "Copy for Claude Code" }));
  expect(writeText).toHaveBeenCalledWith(expect.stringContaining(`Bearer ${useAgents.getState().view!.token}"`));
  expect(screen.getByRole("button", { name: "Copied for Claude Code" })).toHaveTextContent("Copied");
  expect(screen.getByText("No agent has connected yet")).toBeInTheDocument();
});

it("in Moss Dev, tells agents its own name, so it never replaces the installed Moss", async () => {
  useAppIdentity.setState(mossDev);
  const user = setupUser();
  await show(demoAgents({ on: true }));
  expect(screen.getByLabelText("Connection for Claude Code").textContent).toContain(`--scope user moss-dev ${demoUrl}`);
  await user.click(screen.getByRole("radio", { name: "Codex" }));
  expect(screen.getByLabelText("Connection for Codex").textContent).toContain("[mcp_servers.moss-dev]");
  useAppIdentity.setState(installedMoss);
});

it("'Copied' goes back to 'Copy' after a moment", async () => {
  vi.useFakeTimers({ shouldAdvanceTime: true });
  try {
    const user = setupUser({ advanceTimers: vi.advanceTimersByTime });
    await show(demoAgents({ on: true }));
    await user.click(screen.getByRole("button", { name: "Copy for Claude Code" }));
    expect(screen.getByRole("button", { name: "Copied for Claude Code" })).toBeInTheDocument();
    await act(async () => vi.advanceTimersByTime(2000));
    expect(screen.getByRole("button", { name: "Copy for Claude Code" })).toBeInTheDocument();
  } finally {
    vi.useRealTimers();
  }
});

it("each agent has its own snippet; Claude Desktop runs Moss as the bridge", async () => {
  const user = setupUser();
  await show(demoAgents({ on: true }));
  const tabs = screen.getByRole("radiogroup", { name: "Agent" });
  expect(within(tabs).getAllByRole("radio").map((r) => r.textContent)).toEqual(["Claude Code", "Codex", "Claude Desktop", "Cursor", "Other"]);
  await user.click(within(tabs).getByRole("radio", { name: "Codex" }));
  expect(screen.getByLabelText("Connection for Codex")).toHaveTextContent("[mcp_servers.moss]");
  expect(screen.getByText(/~\/\.codex\/config\.toml/)).toBeInTheDocument();
  await user.click(within(tabs).getByRole("radio", { name: "Claude Desktop" }));
  expect(screen.getByLabelText("Connection for Claude Desktop")).toHaveTextContent(demoBridge);
  await user.click(screen.getByRole("button", { name: "Copy for Claude Desktop" }));
  expect(JSON.parse(writeText.mock.calls[0][0] as string).mcpServers.moss.env.MOSS_MCP_TOKEN).toBe(useAgents.getState().view!.token);
  await user.click(within(tabs).getByRole("radio", { name: "Cursor" }));
  expect(screen.getByLabelText("Connection for Cursor")).toHaveTextContent('"headers"');
  // Choosing another agent forgets what was copied.
  expect(screen.getByRole("button", { name: "Copy for Cursor" })).toBeInTheDocument();
});

it("Other gives the URL and the token to copy one at a time", async () => {
  const user = setupUser();
  await show(demoAgents({ on: true }));
  await user.click(screen.getByRole("radio", { name: "Other" }));
  expect(screen.getByText(demoUrl)).toBeInTheDocument();
  await user.click(screen.getByRole("button", { name: "Copy URL" }));
  expect(writeText).toHaveBeenLastCalledWith(demoUrl);
  await user.click(screen.getByRole("button", { name: "Copy token" }));
  expect(writeText).toHaveBeenLastCalledWith(useAgents.getState().view!.token);
  expect(screen.getByRole("button", { name: "Copied token" })).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Copy URL" })).toBeInTheDocument();
});

it("New token asks first, then replaces the token", async () => {
  const user = setupUser();
  await show(demoAgents({ on: true }));
  const before = useAgents.getState().view!.token;
  await user.click(screen.getByRole("button", { name: "New token" }));
  expect(screen.getByText("Agents using the old token stop working until you paste the new one.")).toBeInTheDocument();
  expect(useAgents.getState().view!.token).toBe(before);
  await user.click(screen.getByRole("button", { name: "Yes, make a new token" }));
  expect(useAgents.getState().view!.token).not.toBe(before);
  expect(screen.getByRole("button", { name: "New token" })).toBeInTheDocument();
});

it("says an agent is connected now, or when it last was", async () => {
  await show(demoAgents({ on: true, lastUsed: NOW - 10_000 }));
  expect(screen.getByText("An agent is connected now")).toBeInTheDocument();
});

it("says when an agent last connected a while ago", async () => {
  await show(demoAgents({ on: true, lastUsed: NOW - 3 * 3_600_000 }));
  expect(screen.getByText("An agent last connected 3 h ago")).toBeInTheDocument();
});

it("turning it off hides the connection", async () => {
  const user = setupUser();
  await show(demoAgents({ on: true }));
  await user.click(screen.getByRole("switch", { name: "Connect your agents" }));
  expect(screen.queryByRole("radiogroup", { name: "Agent" })).not.toBeInTheDocument();
});

it("says why when it can't start, or can't copy", async () => {
  const user = setupUser();
  await show({ ...demoAgents(), setOn: () => Promise.reject({ message: "couldn't start the server: address in use" }) });
  await user.click(screen.getByRole("switch", { name: "Connect your agents" }));
  expect(screen.getByText("couldn't start the server: address in use")).toBeInTheDocument();
  useAgents.setState(pristineAgents);
});

it("says so when the clipboard refuses", async () => {
  const user = setupUser();
  writeText.mockRejectedValue(new Error("denied"));
  await show(demoAgents({ on: true }));
  await user.click(screen.getByRole("button", { name: "Copy for Claude Code" }));
  expect(screen.getByText("Couldn’t copy. Select the text and copy it yourself.")).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Copy for Claude Code" })).toBeInTheDocument();
});

it("has a key for each action, shown on screen", async () => {
  const user = setupUser();
  await show();
  await user.keyboard("a");
  expect(screen.getByRole("switch", { name: "Connect your agents" })).toHaveAttribute("aria-checked", "true");
  expect(screen.getByRole("button", { name: "Copy for Claude Code" })).toHaveTextContent("C");
  expect(screen.getByRole("button", { name: "New token" })).toHaveTextContent("N");
  expect(screen.getByText("1 – 5")).toBeInTheDocument();
  await user.keyboard("3");
  expect(screen.getByRole("radio", { name: "Claude Desktop" })).toHaveAttribute("aria-checked", "true");
  await user.keyboard("c");
  expect(writeText).toHaveBeenLastCalledWith(expect.stringContaining("MOSS_MCP_TOKEN"));
  await user.keyboard("5");
  await user.keyboard("c");
  expect(writeText).toHaveBeenLastCalledWith(useAgents.getState().view!.url);
  // Digits past the tabs do nothing.
  await user.keyboard("9");
  expect(screen.getByRole("radio", { name: "Other" })).toHaveAttribute("aria-checked", "true");
  const before = useAgents.getState().view!.token;
  await user.keyboard("n");
  expect(screen.getByRole("button", { name: "Yes, make a new token" })).toBeInTheDocument();
  await user.keyboard("{Escape}");
  expect(screen.getByRole("button", { name: "New token" })).toBeInTheDocument();
  await user.keyboard("nn");
  expect(useAgents.getState().view!.token).not.toBe(before);
  await user.keyboard("a");
  expect(screen.getByRole("switch", { name: "Connect your agents" })).toHaveAttribute("aria-checked", "false");
});

it("while off, only A does anything; and keys typed in a field or with modifiers are left alone", async () => {
  const user = setupUser();
  await show();
  await user.keyboard("c3n{Escape}");
  expect(writeText).not.toHaveBeenCalled();
  expect(useAgents.getState().asking).toBe(false);
  await user.keyboard("{Meta>}a{/Meta}{Alt>}a{/Alt}");
  expect(screen.getByRole("switch", { name: "Connect your agents" })).toHaveAttribute("aria-checked", "false");
  render(<input aria-label="Somewhere to type" />);
  await user.click(screen.getByLabelText("Somewhere to type"));
  await user.keyboard("a");
  expect(screen.getByRole("switch", { name: "Connect your agents" })).toHaveAttribute("aria-checked", "false");
});
