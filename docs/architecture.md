# How Moss works

A map of Moss for people reading or changing the code. Each section says what the code does today.

## The parts

Moss is a Tauri 2 desktop app. The UI is React in `src/`. Everything else is Rust in `crates/`, plus a small Swift helper in `apple/`.

| Part | What it does |
|---|---|
| `moss-core` | Shared types (threads, messages, drafts). They match `src/data/types.ts`. |
| `moss-store` | The local SQLite database: mail, threading, keyword search, drafts, sends, queues. |
| `moss-providers` | Talks to mail servers: IMAP and SMTP, and Resend's API. |
| `moss-accounts` | Sign-in (passwords, OAuth, API keys), server discovery, and where secrets live. |
| `moss-engine` | Ties store, providers and accounts together: sync, actions, sending, drafts, backups. |
| `moss-ai` | The local AI: picks models for the Mac, downloads them, runs them. |
| `moss-extract` | Reads text inside attachments (PDF, images, Office files) on the Mac. |
| `moss-mcp` | The MCP server that lets your own agents use Moss. |
| `src-tauri` | The desktop app. Each Tauri command is one call into the engine or the AI. |
| `apple/` | `moss-apple`, which serves Apple Intelligence through the same local chat endpoint as llama.cpp. |

How they depend on each other:

- `moss-store` and `moss-accounts` use `moss-core`. `moss-providers` stands alone.
- `moss-engine` uses store, providers, accounts and core.
- `moss-mcp` uses the engine.
- `moss-ai` and `moss-extract` don't know about the engine. `src-tauri` connects them.

The engine sends events (new mail, sync progress, problems) to the window as `moss://event`.

## Where your mail is kept

- One SQLite file, `moss.db`, in Moss's data folder (`~/Library/Application Support/app.moss.mail` on a Mac).
- The folder is made private (mode `0700`) at start. Moss refuses to start if it can't do that.
- The database uses WAL mode. Ordinary writes survive a crash. Sends, schedules and other critical changes are written with full sync, so they also survive a power cut.
- Email bodies are compressed in their own table. A setting picks what stays for offline use: the last 90 days of text, all text (the default), or everything including files.
- Files not kept offline stay on the server. Moss fetches them when you open them. Big messages sync without their attachments.
- Keyword search uses SQLite FTS5. Search by meaning uses small vectors from the embedding model, stored in the same database.
- The database is not encrypted. It relies on your Mac's account and the private folder.
- Only one Moss can own a data folder at a time. A second copy refuses to open it.

### Sync

- One background worker per account.
- Newest mail first, then older mail, then IMAP IDLE waits for new mail.
- Progress is saved after every batch, so sync picks up after a restart.
- Gmail: Moss reads the Inbox, Sent, Drafts and its Blocked folder, and fills in conversations from All Mail.
- Other IMAP accounts: Inbox, Sent, Drafts and the Blocked folder. Other folders are not read.
- Resend: received mail comes from Resend's API. Moss keeps its own full copy, because Resend deletes mail after about 30 days.
- Your own copies (in Sent, say) arrive read and Done. Mail in the Inbox that only says it's from you is treated like any other mail: anyone can write your address on the From line.
- Background loops (sync, sorting, the search index, summaries, the attachment reader, send later) start again by themselves if they crash, after a pause that grows with each crash in a row.

## Sending

Every send is recorded in the database before it can go. The record has one of these states:

| State | Meaning |
|---|---|
| `held` | Saved and waiting: for Undo, or for its scheduled time. |
| `claimed` | Moss has started handing it to the server. |
| `accepted` | The server took it. Moss is still saving the local Sent copy. |
| `complete` | Sent and saved locally. |
| `uncertain` | Moss can't tell whether the server took it. |
| `cancelled` | Undone or unscheduled before it went. |

The guarantees:

- The exact email (text, files, Message-ID) is frozen when you press Send. Later edits can't change it.
- Only a `claimed` record may cross to the network. Claiming is one database write, so the same email can't be sent twice by two parts of Moss.
- Sending the same operation again never makes a new email. It returns the existing record.
- If the server clearly refuses (before it took anything), the email goes back to Drafts.
- If the connection drops after the email was handed over, the record becomes `uncertain`.
- If Moss stops while a record is `claimed`, it becomes `uncertain` on the next start.
- An `uncertain` email is never sent again by Moss on its own. A lost reply can't tell Moss whether the email went, and a second try could send it twice.
- An `accepted` email is never sent again either. Moss only retries saving its local copy, with a growing wait (30 seconds up to 5 minutes).

### Reviewing a send

Drafts shows a list of sends that need you: `uncertain` ones, `accepted` ones whose local copy failed, and unsent drafts Moss could not read back.

- An `uncertain` send tells you to check Sent before sending again.
- You can keep a separate copy in Drafts, or discard the saved files.
- Neither choice sends anything.

### Undo

- After Send, the email waits 6 seconds (`held`) so Undo (`⌘Z`) can take it back.
- The wait is a timer in the desktop app (`src-tauri/src/outbox.rs`). The database decides whether it may go: Undo writes the cancellation first.
- Quitting Moss during the wait sends the email at once. Moss waits up to 30 seconds for those sends to finish.
- If Moss stops during the wait (a crash or force quit), the email goes back to Drafts. It is not sent. Moss says so once, at the next launch.

### Send later

- A scheduled email waits on this Mac (the `scheduled` table). Mail servers can't send at a time.
- The engine sends it at its time while Moss is open. It checks at least every 30 seconds, so a Mac that slept still sends soon after waking.
- If Moss was closed at that time, it goes soon after Moss opens again, and Moss says it was late.
- A failed try waits and tries again: 1, 2, 5, 15, 30 minutes, then every hour.
- A scheduled email that became `claimed` or `uncertain` is never retried.
- Each schedule has an operation ID. A late Undo or reschedule can't change a newer schedule of the same draft.

## Drafts

- Drafts save as you type. Each saved draft has a revision (an opaque token from the store).
- A save must name the revision it started from. If the draft changed since (an agent, another device, another window), the save is refused. Nothing is overwritten.
- When your edit can't be saved, Moss keeps it as a private recovery copy. Recovery copies are listed in Drafts and are never sent or synced until you choose one.
- Drafts sync to the account's Drafts folder, so they show on your phone. Resend drafts stay on the Mac.
- The server copy has no signature in its text. A signature you chose or removed in Moss rides along in an `X-Moss-Signature` header, so another Mac keeps your choice.
- A draft written elsewhere (webmail, a phone) gets the signature a new one would get, when it comes in. If its text already ends with one of your saved signatures (exactly, line for line; a draft with only HTML is read with a line for each block, as Gmail web writes them), that one comes out of the text and becomes its signature, so it isn't added twice.
- When the server copy and your copy both changed, Moss keeps both as separate drafts and asks you to review Drafts.
- Quitting Moss waits for pending saves. It tells you what is still unsaved instead of quitting.

## Server changes

Done, read, delete, move, block and similar actions change your mail locally at once. The server change goes into a queue (the `outbox` table) and runs later, in order, per account.

- Each change has a persistent ID and a state: `queued`, `claimed` or `blocked`.
- A change is `claimed` in the database before Moss talks to the server.

What happens when a try doesn't simply work (the table at the top of `crates/moss-engine/src/server_queue.rs`):

| The server... | Then |
|---|---|
| Didn't answer: network, a stall, a rate limit, a sign-in to renew | Back in the queue, in its place. Tried again on the next sync. |
| Said no, and the mail is gone from it | Dropped: there is nothing left to change. |
| Refused it any other way | `blocked` until you pick Try again or Discard on the notice. |

- A change left mid-way by a crash or force quit is tried again at the next start.
- Trying again is safe: each try looks at the server as it is now. Flags are set, not toggled, and a message already moved is no longer there to move.
- A change Moss can't read, or one that is too large, is also `blocked`. It can only be discarded.
- A `blocked` change holds back later changes for that account. The inbox shows a notice with the reason and how many parts the server confirmed.
- Reading new mail goes on while changes wait.
- Known limit: a server without IMAP MOVE copies then deletes. If the connection drops in between, you can end up with a second copy. Nothing is lost.

## Accounts

- Adding or signing in again saves the account and its connection in one transaction.
- Removing an account is refused while it has an unsent or uncertain send, or scheduled mail. The message says which.
- When only server changes are in the way, Moss asks first: "N changes haven't reached the server and will be lost." Saying yes removes the account and those changes together.
- Otherwise Moss deletes the account's local mail and settings in one transaction, then removes its Keychain item. If the Keychain step fails, Moss keeps a note and retries. It also deletes the copy from before the last update, which still holds that account's mail. See [account-removal.md](account-removal.md).

## Backups

- Settings, Storage makes a backup of the mail database to a file you pick. It shows progress and can be cancelled.
- The backup is a checked SQLite copy. It leaves out settings, sign-ins and the list of Keychain items to clean up.
- Once saved, the backup is read back from where it was saved and checked before Moss calls it done.
- There is no restore yet. A backup can't be opened as a live database by mistake.

### Updating the mail database

- A new Moss updates an older database only while no other Moss has it open. Otherwise it stops with "Another Moss is using this mail. Quit it first." and changes nothing.
- Before the update it keeps a copy beside it: `moss.db.before-update`. A copy from an earlier update is removed first. If the copy can't be made (no space, say), the update goes ahead.
- The copy is deleted on the next launch that needs no update, and when an account is removed.
- Work files left by a backup or update that stopped partway are cleared at launch.

More in [backup-recovery.md](backup-recovery.md).

## Secrets

- Release builds keep passwords, OAuth tokens, API keys and the MCP token in the macOS Keychain. If the Keychain fails, Moss says so. It never falls back to a file.
- Debug builds use an encrypted file in the data folder, with its key in a second file beside it. macOS ties Keychain access to the exact binary, so every rebuild would ask again.
- A release build that finds a sign-in in that file copies it to the Keychain, reads it back, then removes it from the file.
- Secrets never go into the database or logs.

More in [local-credentials.md](local-credentials.md).

## Files you receive

- When you open an attachment, Moss saves it with the macOS quarantine mark first, then opens it. macOS then treats it like a download.
- File names lose control and text-direction characters, so a name can't pretend to be another type.
- Text inside attachments is read on the Mac, within size limits. See [attachment-processing.md](attachment-processing.md).

## The email frame

An HTML email is shown in a frame that can't run its own code or reach Moss.

- **Sanitiser.** DOMPurify cleans the HTML. Forms, frames, objects, media, SVG, `<base>`, `<meta>` and `<link>` are removed, as are attributes that could send a click elsewhere.
- **Content Security Policy.** The frame's document allows no scripts, no forms, no fonts from the web and no network, except images once they are allowed.
- **Sandbox.** The frame is sandboxed. Moss's own listeners size it, pass keys on and catch link clicks. The policy still blocks the email's scripts.
- **No referrer.** Image loads send no referrer.
- **Links.** Only `http`, `https` and `mailto` links open, and they go to your browser. Other windows stay on Moss's own pages (`src-tauri/src/links.rs`).
- **Remote images.** Load by themselves for familiar senders, but only when the account's mail server confirmed the sender (`crates/moss-store/src/authres.rs`). Moss knows how to read that for Gmail, Microsoft, iCloud and Fastmail accounts; other accounts always ask first. "Just this once" loads them for any email. Settings, Appearance can ask for every message. Images that look like trackers are removed, but that check is a guess.

`pnpm test:native-email` checks these documents in WebKit. See [native-email-security.md](native-email-security.md).

## Local AI

- Models run in llama.cpp's `llama-server`, on `127.0.0.1`, behind a random key. On a Mac with Apple Intelligence, `moss-apple` can do the writing instead.
- Nothing downloads until you ask, or first use an AI feature. Files are pinned to exact versions and checked by SHA-256.
- Big downloads wait while the battery is below 20% or on a phone hotspot.
- Each model starts the first time a job needs it.
- A model left unused for 10 minutes is stopped, to free memory.
- On battery, background work waits: meaning search indexing and summaries nobody asked for. What you ask for always runs.
- Ask Moss can search, read and draft. It has no tool that sends: you press Send.

## MCP server for your agents

Off until you turn it on in Settings, AI.

- Streamable HTTP on `127.0.0.1` only, at `http://127.0.0.1:47217/mcp` (or a free port if that one is taken).
- Every request needs `Authorization: Bearer <token>`. The token is random, kept in the Keychain, and can be replaced, which stops the old one at once.
- Requests that carry an `Origin` header (a web page) or name another `Host` are refused.
- Nothing an agent does is logged.

What agents can do: list and read mail, search, mark Done or read, remind, move, delete to Trash, block and accept senders, write and change drafts, send now or later, cancel a scheduled email, save an attachment from mail and get its path.

Sending from an agent:

- `send` takes an optional `request_id`. A retry with the same value never sends twice, and says what became of the first try.
- Results name the draft (its id), so an email that wasn't sent can be found and sent later.
- The bridge waits up to 15 minutes for an answer, longer than a slow mail server may take.

What agents can't do: read files on your Mac. Moss never reads a local path an agent gives it. To attach a file, the agent sends its contents (at most 1 MB in total), or uses a file already attached in Moss.

## Network limits

A server that stops answering fails instead of hanging.

- **IMAP:** 30 seconds to connect and to sign in. After that, a command fails if nothing arrives for 2 minutes. A download that keeps arriving is never cut. IDLE turns the check off while it waits.
- **SMTP:** 60 seconds to connect. Then up to 10 minutes of silence, the limit the SMTP standard (RFC 5321) allows for the final reply. Silence after the email went out makes the send `uncertain`.
- **Resend:** 10 seconds to connect, 60 seconds per read and per API call.
- **Sign-in and discovery:** 10 seconds to connect, 15 seconds per request.
- **Model downloads:** 15 seconds to connect, 60 seconds of silence per read.

## Network hosts

Moss has no server of its own. It talks to:

- Your mail providers (IMAP, SMTP, Resend's API).
- Google and Microsoft sign-in, for OAuth accounts. Gmail's API, only to list "Send mail as" addresses when you use your own Google OAuth client.
- For an address it doesn't know: the domain's own autoconfig file, Mozilla's ISPDB (`autoconfig.thunderbird.net`) and DNS.
- Hugging Face and GitHub, for model downloads you ask for.
- GitHub, for update checks (every 6 hours) and updates.
- Image hosts, when you allow images in an email.

## Decisions

- **Uncertain sends are never retried by Moss.** A stable Message-ID doesn't stop a server from delivering a second copy.
- **Server changes are tried again only when that is safe.** No answer means try again; a refusal waits for you.
- **The database decides, not the timer.** The Undo timer only says when. Claims and cancellations live in the database.
- **MCP agents may send without asking in the app.** That is what connecting an agent is for. They may not attach files from your Mac: reading files is a separate power Moss doesn't give.
- **No automatic history rewriting or credential deletion.** Moss deletes a Keychain item only by its exact saved reference, after the account is gone. It never guesses item names.
- **Draft conflicts keep both versions.** A newer timestamp is never permission to overwrite.
- **Backups leave out sign-ins and settings.** A backup file may be stored in a synced folder.
- **No database encryption yet.** It needs evidence that it helps more than it costs.
