# Your own agents: Moss's MCP server

Agents you already use (Claude Code, Codex, Claude Desktop, Cursor and others) connect to Moss and manage your mail: read, sort, remind, draft and send. It is the path for people who want a strong cloud model to run their inbox. Moss's own AI stays on the Mac; this is the choice to let another one in.

## The decision

**Streamable HTTP on `127.0.0.1`, with a bearer token. Plus a small stdio bridge for Claude Desktop.**

- Moss is a running app, so the server lives inside it and uses the same engine the window uses. No second process, no copy of the database.
- Claude Code, Codex and Cursor all take a URL with a header. That is one line or a few lines of config each.
- Claude Desktop's config file only starts programs (stdio). Its remote "connectors" are reached from Anthropic's cloud, so they can't see `127.0.0.1`. For Claude Desktop, Moss's own app runs as the bridge: `Moss mcp <url>`, with the token in `MOSS_MCP_TOKEN`. It passes each line to the running server and writes back the answer. No Node, no third-party package.
- Plain stdio for everyone was the other choice. It would mean a separate process that opens the database while the app has it open, or a bridge for every agent. HTTP with one bridge is simpler.

## How it works

- Crate: `crates/moss-mcp`, on the official Rust SDK (`rmcp` 3.9) and `axum`. Started from `src-tauri` (`src-tauri/src/mcp.rs`).
- Address: `http://127.0.0.1:47217/mcp`. If that port is taken, Moss picks a free one and keeps it, so what you pasted keeps working.
- Off until you turn it on in Settings, AI, Your own agents. Off stops the server: connections are refused.
- Every request needs `Authorization: Bearer <token>`. The token is 32 random bytes, kept in the Keychain (item `mcp`). New token replaces it and the old one stops working at once.
- Only loopback `Host` names are accepted, and any request with an `Origin` header is refused, so a web page can't reach it.
- Stateless, with plain JSON answers (no sessions, no event streams for simple calls).
- Nothing is logged, so no mail lands in a log.
- Settings (on, port) are in `mcp.json` beside the database.

## Tools

All go through `moss-engine`, the same calls the window makes, and tell the window to reload.

| Tool | Does |
|---|---|
| `list_accounts` | Accounts: id, address, name, and the other addresses each sends from |
| `list_inbox` | Open threads, newest first, by split and account. Hides reminders not due yet. 25 by default, 100 at most |
| `get_thread` | One thread with every email: from, to, cc, date, text. Attachments by name only, never their bytes. Text over 20,000 characters is cut. Reading doesn't mark it read |
| `get_attachment` | Saves a file from a thread on this Mac, by name, and says where |
| `search_mail` | Words, then meaning when the search model is on this Mac |
| `list_drafts` | Unsent drafts |
| `list_done` / `list_blocked` | Done mail, and mail from blocked senders, newest first |
| `list_scheduled` / `cancel_scheduled` | Emails set to send later; cancelling puts one back in Drafts |
| `find_contacts` | People written to or heard from, by part of a name or address |
| `sync_now` | Checks for new mail now |
| `mark_done` | Done (archived on the server), or back to the inbox |
| `mark_read` | Read or unread, here and on the server |
| `remind_me` | Hide until a time, then back at the top; or stop reminding |
| `remind_if_no_reply` | Bring it back if no one answers the user's latest email |
| `move_thread` | To another split for good (Important stars it), or back to Moss's sorting |
| `delete_thread` / `restore_thread` | To Trash and back |
| `block_sender` / `accept_sender` / `unblock_sender` | By address or whole domain |
| `create_draft` / `update_draft` / `delete_draft` | Write, change or delete a draft without sending |
| `send` | A new email, a reply, reply all, a forward (with its files), or a draft by id. `send_at` sends later. |

Writing works as in the window: Bcc, `from` one of the account's addresses (a reply goes from the address the email came to), files already attached to a draft in Moss (use `draft_id`), or up to 1 MB of file contents supplied by an external agent, and the signature Moss picks for the address and kind of email (or one by name, or "none").

### Sending

- `send` sends straight away through the engine, like Gmail's connector in Codex. No ask in Moss.
- The rule is in the tool's description and the server's instructions: only call `send` when the user clearly asked to send that email; for a draft use `create_draft`; if it isn't clear, ask the user first.
- A failed send keeps the email as a draft and says its id, so nothing is lost.
- The sent email is in Sent as usual, and the result says "Sent".
- `send_at` uses Send later: it waits in Moss and goes at its time, and the user can change or cancel it there.

### Moss's own chat uses these tools too

Ask Moss, on the writing model on this Mac, gets the same tools without the server (`MossTools::chat_specs` / `chat_run`, see [ask-agent.md](ask-agent.md)). The differences: it has its own search, read and people tools, so those here are left out; and `send` is `write_email`, which never sends: the email waits under the answer for the user to press Send.

### Not done (would need new work)

- **Undo send** for agent sends: Undo is a hold in the app plus a toast in the window. An agent's send goes at once. Adding it needs the window to show a toast for sends it didn't start.
- **Summaries**: they need the writing model, which lives in the app (`moss-ai`), not the engine. An agent can summarise what `get_thread` returns itself.
- **Attachment bytes in results**: `get_attachment` saves the file on this Mac and says where, instead.

## Connecting each agent

Settings, AI, Your own agents shows each of these with your real address and token, ready to copy (`C`). Keys there: `A` on or off, `1` to `5` pick the agent, `C` copy, `N` new token.

- **Claude Code** (Terminal):
  `claude mcp add --transport http --scope user moss http://127.0.0.1:47217/mcp --header "Authorization: Bearer <token>"`
- **Codex** (`~/.codex/config.toml`):
  ```toml
  [mcp_servers.moss]
  url = "http://127.0.0.1:47217/mcp"
  http_headers = { "Authorization" = "Bearer <token>" }
  ```
- **Claude Desktop** (Settings, Developer, Edit Config):
  ```json
  { "mcpServers": { "moss": {
      "command": "/Applications/Moss.app/Contents/MacOS/Moss",
      "args": ["mcp", "http://127.0.0.1:47217/mcp"],
      "env": { "MOSS_MCP_TOKEN": "<token>" } } } }
  ```
- **Cursor** (`~/.cursor/mcp.json`):
  ```json
  { "mcpServers": { "moss": { "url": "http://127.0.0.1:47217/mcp",
      "headers": { "Authorization": "Bearer <token>" } } } }
  ```
- **Other**: the URL, and the token as a Bearer token.

## Tried with real clients

On made-up mail (`crates/moss-mcp/tests/real_client.rs`, run with `--ignored`):

- Claude Code (`claude -p` with `--mcp-config`, HTTP): listed the 6 tools, searched, read the thread, saved a reply draft.
- Claude Code through the bridge, with Claude Desktop's exact config shape: listed the inbox, saved a reply draft.
- Codex (`codex exec` with the `config.toml` lines above): called `list_accounts`.
- Cursor: not installed here, so not tried. Its config is the documented `url` plus `headers`.

## Privacy

Agents you connect can read, manage and send your email. A cloud agent sends what it reads to its company. Moss says so, plainly, under the switch. Nothing goes anywhere until you turn it on and paste the connection into an agent.

## Attachment authorization

Moss never reads a path supplied by an AI tool. External agents can supply `attachments`
as `{ "name": "notes.txt", "content_base64": "aGk=" }`, with at most 1 MB of decoded
contents in 100 files per call. This uses only bytes the agent already holds. Larger files
must be attached in Moss's composer; an agent can then send that draft by `draft_id`.
Forwarding keeps attachments from the existing mailbox.

Ask Moss cannot supply local paths or attachment contents, even if its model invents
those arguments. The backend rejects them before saving or sending a draft. This matters
because drafts on IMAP accounts sync to the provider without a Send click. Files the user
already attached to a draft remain available when that draft is edited or sent.

### Scheduled operation identities

`list_scheduled` returns `scheduledId`, immutable `operationId` and `revision` for each row. Pass both `scheduled_id` and `operation_id` to `cancel_scheduled`. The operation must be an active schedule or a retained terminal schedule outcome. Unknown operation IDs are rejected; agents cannot create early cancellation records. Desktop Undo retains its separate early-cancellation behavior.

Only a cancellation that actually restores mail returns `draftId` and the "back in Drafts" result. A repeated cancellation returns `status: "cancelled"` with `alreadyCancelled: true` and a note that other schedules are unchanged. `sending`, `inDelivery` and `completed` do not mean cancellation succeeded. Revision is informational for this tool; changing the time does not change which operation is being cancelled. This changes the required cancellation arguments, so agents must refresh their tool schema.
