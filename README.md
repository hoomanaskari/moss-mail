# Moss

**Calm, keyboard-first email with private AI.** Free and open source (MIT).

<p align="center">
  <a href="https://github.com/hoomanaskari/moss-mail/releases/latest">
    <img src="https://img.shields.io/badge/Download_for_Mac-DMG-black?style=for-the-badge&logo=apple&logoColor=white" alt="Download for Mac" />
  </a>
</p>

Moss is an email client that has AI, not an AI app that reads your email. It's built to be easy on people with dyslexia and ADHD: one clear next step, big readable text, and every shortcut shown on screen. The AI runs on your Mac, so your email stays there.

> **Status: early.** Moss 0.1 is the first public version, for **macOS on Apple silicon** (M1 or newer), macOS 13 or later.
> The code for Windows and Linux is written, but it hasn't been built or tested yet. Those versions are coming if Moss finds its people.

## Download

1. Get the newest `Moss_<version>_aarch64.dmg` from [Releases](https://github.com/hoomanaskari/moss-mail/releases/latest).
2. Open it and drag Moss to Applications.
3. Open Moss and add your first account.

Moss keeps itself up to date. It looks for a new version when it opens and every 6 hours. When there is one, **Update to Moss** and its number shows at the top of every screen: click it and Moss downloads it, installs it and restarts. You can also look now: choose **Moss**, then **Check for Updates…** in the menu bar, or press `⌘K` and choose **Check for updates**.

## What it does

- **All your accounts in one inbox**, or one at a time (`⌘0` to `⌘9`).
- **A split inbox** that sorts mail for you: Important, Calendar, Newsletters, Receipts and Other.
- **Done** (`E`) instead of archive folders, and **inbox zero** when you're through.
- **Remind me** (`H`): the email comes back later today, tomorrow, next week, or when you pick.
- **Send later** (`⌘⇧↵`), drafts that show up on your phone too, and signatures that pick themselves.
- **Block senders** (`B`): new senders get a banner, and blocked mail goes out of sight.
- **Search** (`/`) by word or by meaning.
- **AI on your Mac**: summaries (`S`), quick replies, help writing, and Ask Moss (`⌘J`): ask about your mail, or tell it what to do (mark Done, remind, reply, forward, block), even on many emails at once. It never sends: you press Send.
- **Your own agents**: Claude Code, Codex, Claude Desktop, Cursor and others can manage your mail through MCP: read, sort, remind, draft and send.

## Adding your accounts

Moss talks straight to your email provider. There is no Moss server in between.

| Provider | How you connect | What you need |
|---|---|---|
| **Gmail** | App password | 2-Step Verification on your Google account. Moss shows each step. [More on Gmail](docs/first-run.md) |
| **Outlook / Microsoft 365** | Sign in in your browser | Nothing else. Work and school accounts may need their admin to approve Moss once. |
| **iCloud Mail** | App password | An app-specific password from [account.apple.com](https://account.apple.com). For a custom domain, also your @icloud.com address. |
| **Yahoo** | App password | An app password from your Yahoo account security page. |
| **Fastmail** | App password | An app password from Fastmail's settings. |
| **Other (IMAP)** | Email and password | Your server's IMAP and SMTP details, if Moss can't find them. |
| **Resend** | API key | A key with **Full access** that covers **all domains** ([resend.com/api-keys](https://resend.com/api-keys)). Then pick which domains Moss receives mail for. |

A few things worth knowing:

- **Why Gmail needs an app password:** Google only allows one-click sign-in for full mail access after a paid security review. Moss is free, so it uses app passwords. If you'd rather, you can use your own Google OAuth client instead (Moss walks you through it).
- **Resend deletes received mail after about 30 days.** Moss keeps its own copy, so older mail lives only on your Mac. Removing a Resend account from Moss deletes that copy for good.
- **Passwords, keys and sign-ins** are kept in the macOS Keychain, never in the mail database.

## Your privacy

All the AI runs on your Mac. Moss has no server of its own, no analytics, no tracking and no account of its own.

What stays on your Mac:

- Your email as Moss stores it, its search index, summaries and drafts.
- Sorting, search by meaning and everything the built-in AI does. Your mail is never sent to an AI service.
- Your sender rules: who you trust, block or set reminders for. (Blocking also moves that sender's mail to a Blocked folder on your mail server.)

What leaves it, and where it goes:

- **Your mail** goes between your Mac and your own provider (Google, Microsoft, Apple, Resend and so on), as with any mail app. Drafts are saved to your provider's Drafts folder too (not for Resend), so they show on your phone.
- **Signing in** to Outlook, or to Gmail with your own Google OAuth client, opens Microsoft's or Google's sign-in page in your browser.
- **Finding server settings** for an address Moss doesn't know: Moss asks the domain's own setup file, Mozilla's settings list (`autoconfig.thunderbird.net`) and DNS.
- **Images in emails** load from the web servers the email points to. By default they load by themselves only for familiar senders, and only when your mail server confirmed the sender (Gmail, Microsoft, iCloud and Fastmail accounts; other accounts always ask first). **Just this once** loads them for any email. Settings, Appearance offers **Ask for every message**. Moss removes images that look like trackers and sends no referrer, but it can't catch every tracker or hide your IP address when images load.
- **The AI models** download once, from Hugging Face and GitHub, when you ask for them.
- **Update checks** go to GitHub every 6 hours. GitHub sees normal connection details such as your IP address. No email content or account details are sent.
- **Links** you click open in your browser.
- **Your own agents, only if you turn them on.** Settings, AI, Your own agents lets an agent such as Claude Code, Codex, Claude Desktop or Cursor connect to Moss, on this Mac only (MCP on `127.0.0.1`, with a token kept in the Keychain). Agents you connect can read, manage and send your email. A cloud agent sends what it reads to its company.

## AI on your Mac

Moss downloads its AI models when you ask, and picks the best ones for your Mac's memory. You can change or remove them in Settings, AI.

On a Mac with **Apple Intelligence** (macOS 27, Apple silicon), Apple's own on-device model can do the writing instead of a Gemma model: it's built into macOS, so there's nothing to download for it, and it's quick. Gemma 4 E4B writes better, and handles long requests in Ask Moss more reliably; you choose in Settings, AI. Moss reaches Apple's model through `moss-apple`, a small Swift helper that ships inside Moss.app (its code is in `apple/`).

| Model | What it does | Size |
|---|---|---|
| [llama.cpp](https://github.com/ggml-org/llama.cpp) | Runs the models, using your Mac's GPU | 16 MB |
| [d3-lite](https://huggingface.co/ggml-org/vllm-d3-lite-GGUF) | Sorts mail and decides what needs you | 1.1 GB |
| [EmbeddingGemma 300M](https://huggingface.co/ggml-org/embeddinggemma-300M-GGUF) | Search by meaning | 0.3 GB |
| Apple Intelligence | Writes, on a Mac that has it: built into macOS | None |
| [Gemma 4 E2B](https://huggingface.co/unsloth/gemma-4-E2B-it-GGUF) | Writes: summaries, replies, Ask Moss. For Macs under 16 GB | 3.1 GB |
| [Gemma 4 E4B](https://huggingface.co/unsloth/gemma-4-E4B-it-GGUF) | Writes, better. For 16 GB or more | 5.0 GB |
| [Gemma 4 26B](https://huggingface.co/unsloth/gemma-4-26B-A4B-it-GGUF) | Writes, best. For 48 GB or more | 16.9 GB |

Each file is pinned to an exact version and checked before use. Each model's licence is on its page.

## Keys

| Key | Does |
|---|---|
| `↓` / `↑` | Next or previous email |
| `↵` | Open |
| `Esc` | Go back |
| `E` | Done, or not done again |
| `H` / `⇧H` | Remind me / Remind me if no reply |
| `M` | Move to another tab, such as Important |
| `R` / `⇧R` / `F` | Reply / Reply all / Forward |
| `C` | New email |
| `⌘↵` / `⌘⇧↵` | Send / Send later |
| `S` | Summarise the open email |
| `⌘J` | Ask Moss |
| `⌘K` | Ask AI or run any command |
| `/` | Search |
| `B` / `A` | Block / Accept a new sender |
| `⌘0` to `⌘9` | All accounts, or one |
| `⌘Z` | Undo |
| `?` | Every shortcut |

## Known limits

- **No restore from a backup yet.** Moss can make a backup of your mail database, but it can't load one back in. A backup leaves out your sign-ins and settings, and mail that lives only on the server.
- **Only some folders sync.** For iCloud, Outlook, Yahoo, Fastmail and other IMAP accounts, Moss reads your Inbox, Sent, Drafts and its own Blocked folder. Other folders stay on the server and don't show up in Moss.
- **The whole inbox loads at once.** There is no paging yet, so a very large inbox takes longer to open.
- **Agents you connect can send mail on their own.** An agent connected through MCP can send email without asking you first. It can't attach files from your Mac. Connect only agents you trust.
- **Send later needs Moss running.** An email set for later goes when Moss is open at that time. If Moss is closed, it goes soon after you open it again, and says it was late.
- **Quitting during Undo sends at once.** If you quit Moss while an email is waiting for Undo, it is sent straight away.
- **If Moss can't tell whether an email went, it won't try again by itself.** It asks you to check Sent first, so nothing is sent twice.
- **Server changes the server refuses wait for you.** Moss shows a notice with Try again and Discard.

## Coming next

- [ ] **Windows and Linux** builds, tested on real machines
- [ ] **Resend:** show mail you send through Resend from other tools
- [ ] **Export** your mail to a standard `.mbox` file
- [ ] **Outlook for work and school** without asking an admin (needs a verified Microsoft publisher)
- [ ] **Your own split tabs**, described in plain words
- [ ] **Notifications** only for the mail that needs you now
- [ ] **Smarter search** that answers questions about your mail
- [ ] **Connect your other tools to Moss's own AI** (Slack, Stripe, your calendar and more, through MCP), so it can answer with more than your email while everything stays on your Mac. Still to be tested: the models that run on a Mac may not be good enough at this yet.
- [ ] **Voice control**
- [ ] **iPhone and Android**, later

Ideas and requests are welcome in [Issues](https://github.com/hoomanaskari/moss-mail/issues).

## Build it yourself

You need:

- macOS on Apple silicon,
- Node 24 and the pnpm version pinned in `package.json`,
- Rust 1.98 or newer ([rustup.rs](https://rustup.rs)),
- Xcode 27 or newer, whose Swift builds the Apple Intelligence helper in `apple/`.

```bash
pnpm install
pnpm tauri dev      # Moss Dev, the desktop app for trying changes
pnpm dev            # the UI only, in a browser at http://localhost:1420, with demo mail
pnpm app            # build Moss Dev.app in target/release/bundle/macos
```

### Moss and Moss Dev

What you build is **Moss Dev**, a separate app from the Moss you install, so trying a change never touches the mail you use. Both can run at the same time.

| | Moss | Moss Dev |
|---|---|---|
| Made by | `pnpm release` only | `pnpm app`, `pnpm tauri dev` |
| Bundle identifier | `app.moss.mail` | `app.moss.mail.dev` |
| Data folder | `~/Library/Application Support/app.moss.mail` | `~/Library/Application Support/app.moss.mail.dev` |
| Keychain items | service `app.moss.mail` | service `app.moss.mail.dev` |
| Agents (MCP) | port 47217, named `moss` | port 47218, named `moss-dev` |
| Updates | checks and installs them | none |
| Dock icon | Moss | Moss with an orange DEV band, and a Dev mark in the window |

Moss Dev starts empty: add an account to try it. AI models are downloaded again for it, into its own folder. To skip that, clone the installed Moss's models into Moss Dev's folder (an APFS clone takes no extra space, and nothing is written to the installed Moss's folder):

```bash
mkdir -p ~/Library/Application\ Support/app.moss.mail.dev/ai
cp -cR ~/Library/Application\ Support/app.moss.mail/ai/models ~/Library/Application\ Support/app.moss.mail/ai/runtime ~/Library/Application\ Support/app.moss.mail.dev/ai/
```

To try Moss Dev on a **copy** of your mail, quit Moss first (so the copy is whole), copy its folder, then start Moss Dev with `MOSS_DATA_DIR`:

```bash
cp -cR ~/Library/Application\ Support/app.moss.mail /tmp/moss-copy
rm -rf /tmp/moss-copy/ai/run /tmp/moss-copy/mcp.json
MOSS_DATA_DIR=/tmp/moss-copy "target/release/bundle/macos/Moss Dev.app/Contents/MacOS/Moss"
```

Your mail is there, but every account asks you to sign in again: sign-ins live in the Keychain, not in the folder, and Moss Dev can't reach the installed Moss's. Signing in again in the copy makes Moss Dev's own Keychain items. Moss Dev never opens the installed Moss's own folder, even if `MOSS_DATA_DIR` points there, and the installed Moss ignores `MOSS_DATA_DIR`.

`pnpm app` signs Moss Dev.app with your Developer ID certificate (or `APPLE_SIGNING_IDENTITY`), so the Keychain's "Always Allow" lasts from one build to the next. Without one it signs ad hoc, and macOS asks for your password again after every build.

Debug builds (`pnpm tauri dev`) keep sign-ins in an encrypted file in Moss Dev's data folder, with its key beside it, so the Keychain doesn't ask after every rebuild. Anyone who can read both files can read the sign-ins. Release builds use the Keychain only. When Moss Dev.app finds a sign-in from a debug build, it moves it into its Keychain items. Keep development data and backups private.

The desktop app shows real mail. The browser (`pnpm dev`) and the `?scene=` links always use the demo mailbox.

Moss Dev's icon is drawn by `sh scripts/dev-icon.sh` from `src-tauri/icons/Moss.icon`: run it after changing that icon, and commit what it writes in `src-tauri/icons/dev/`.

## How it's built

- **Rust** for everything heavy, in `crates/`: `moss-core` (shared types), `moss-store` (SQLite and FTS5 search), `moss-providers` (IMAP, SMTP, Resend), `moss-accounts` (sign-in, Keychain), `moss-ai` (the models), `moss-extract` (text inside attachments), `moss-mcp` (the server for your agents) and `moss-engine`, which ties them together: sync, done, remind, send.
- **Tauri 2** for the desktop app, in `src-tauri/`. Each engine call is one command.
- **Swift** for `moss-apple`, in `apple/`: Apple Intelligence behind the same local chat endpoint as llama.cpp, built by `src-tauri/build.rs` and shipped beside Moss.
- **React and TypeScript** for the UI, in `src/`.

How storage, sending, drafts, backups, secrets and the email frame work: [docs/architecture.md](docs/architecture.md).

The UI follows **atomic design**, so every screen is built from small shared parts:

```
src/design-system/
  tokens/      colours, fonts, spacing (one-to-one with design.pen variables)
  atoms/       Kbd, AccountBadge, Avatar, Button, Pill, Icon, ProgressRing…
  molecules/   ThreadRow, AccountTab, ShortcutHint, RecipientChip, Notice…
  organisms/   ThreadList, CommandPalette, InstantReply, ComposerFields…
  templates/   AppFrame, ReadingLayout, ComposeLayout
src/screens/   Inbox, Thread, Compose, Settings and the rest
```

### Design fidelity

`design.pen` is the source of truth: every token, size and colour comes from it. It is a Pencil file kept by the maintainer, not in this repository; its values are in `src/design-system/tokens/`. Dev builds have a link to each designed state, used to compare the app with the design pixel by pixel:

```
http://localhost:1420/?scene=split-inbox&theme=light
```

The full list of scenes is in `src/dev/scenes.ts`.

### Tests

New behaviour starts with a failing test, then the code that makes it pass, and coverage is kept as high as it can go. The rules are in [AGENTS.md](AGENTS.md).

```bash
pnpm test           # UI and release tests (Vitest and Testing Library)
pnpm coverage       # UI and release coverage; full report in coverage/index.html
cargo test --workspace
cargo llvm-cov --workspace --summary-only   # Rust coverage (cargo install cargo-llvm-cov)
swift test --package-path apple             # the Apple Intelligence helper (runs Apple's model where it can)
pnpm test:native-email   # hostile test emails in WebKit, on a Mac
pnpm check          # everything before a push: typecheck, the coverage floors, the helper's tests, native email checks, app build
```

The release-signature tests use disposable keys and need minisign (`brew install minisign` on macOS). There is no CI server: run `pnpm check` before pushing or merging.

### Releases

`pnpm set-version 0.2.0` sets the version everywhere it's kept. `pnpm release` builds, signs and notarizes Moss, makes the DMG and the update, and drafts a GitHub Release. See [docs/releasing.md](docs/releasing.md).

## Readability

- Light and dark themes. Moss follows your system unless you pick one.
- Three reading sizes, in Settings, Appearance.
- Respects "reduce motion".

## Contributing

How to help: [CONTRIBUTING.md](CONTRIBUTING.md).

## Security

Found a security problem? Please don't open a public issue. Open a private security advisory on GitHub: the repository's **Security** tab, then **Report a vulnerability**. More in [SECURITY.md](SECURITY.md).

## Licence

MIT. See [LICENSE](LICENSE).
