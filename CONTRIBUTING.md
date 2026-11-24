# Contributing to Moss

Thanks for wanting to help. Moss is small and early, so the most useful things right now are bug reports from real accounts, and fixes with tests.

## Before you start

- **Bugs and ideas:** open an [issue](https://github.com/hoomanaskari/moss-mail/issues) first, so we can agree on the fix before you spend time on it.
- **Security problems:** please don't open an issue. See [SECURITY.md](SECURITY.md).
- **Platforms:** Moss is built and tested on macOS (Apple silicon). Windows and Linux aren't supported yet, but help testing them is welcome.

## Setting up

```bash
pnpm install
pnpm tauri dev      # Moss Dev: the desktop app, apart from the Moss you use
pnpm dev            # the UI only, in a browser, with demo mail
```

You need Node 24, pnpm, and Rust 1.98 or newer. What you build is Moss Dev, a separate app from the Moss you install: its own data folder, Keychain items and icon, so trying a change never touches your real mail. `pnpm app` builds it as an app. See "Moss and Moss Dev" in the README.

## How we work

The full rules are in [AGENTS.md](AGENTS.md). The short version:

1. **Test first.** Write the test, see it fail for the right reason, then write the code that makes it pass.
2. **Keep coverage as high as it goes.** New code ships with its tests. `pnpm check` fails if coverage drops.
3. **Run `pnpm check` before you open a pull request.** There is no CI server, so this is the check.
4. **Follow the design.** Every colour, size and word on screen comes from `design.pen`, a Pencil file the maintainer keeps outside this repository. Use the design system's tokens and parts, which mirror it. If a screen isn't designed yet, say so in the pull request.

## Writing for Moss

Moss is built for people with dyslexia and ADHD, so the words matter as much as the code:

- Short sentences and everyday words. Say what happened and what to do next.
- One clear next step on each screen.
- Every action has a key, and the key is shown on screen.
- No em dashes in anything people read: use a full stop, a comma or a colon.
- The AI is called "Moss" in the app. Its icon is the teal leaf, never sparkles.

## Pull requests

- One change per pull request, with a title that says what it does for people.
- Say how you tested it, including on which kind of account (Gmail, iCloud, IMAP…) if it touches mail.
- Screenshots for anything you can see.

By contributing, you agree that your work is shared under the [MIT licence](LICENSE).
