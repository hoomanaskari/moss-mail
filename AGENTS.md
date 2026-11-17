# Moss

Rules for anyone (human or agent) changing this repo.

## Test first

Work test-first, in both the UI and Rust:

1. Write the test for the new behaviour or the bug. Run it and see it fail for the right reason.
2. Write the least code that makes it pass.
3. Tidy up with the tests green.

- A test that passes on its first run proves nothing yet: break the code on purpose, check the test goes red, then put the code back.
- Rules that exist in both TypeScript and Rust (e.g. `isExternal` / `Person::is_external`) get the same test cases on both sides.
- Visual fidelity to `design.pen` is still checked with the `?scene=` screenshots, not unit tests.

## Coverage: as high as it can reasonably go

The bar is maximum coverage, not "enough". Every change must leave coverage at least where it was, and new code ships with its tests at 100% of lines and as close to 100% of branches as the code allows.

```bash
pnpm coverage                                  # UI; report in coverage/index.html
cargo llvm-cov --workspace --summary-only      # Rust (cargo install cargo-llvm-cov)
swift test --package-path apple --enable-code-coverage   # the Apple Intelligence helper
```

- `pnpm check` runs both and fails if coverage drops below the floors (`test.coverage.thresholds` in `vite.config.ts`, `--fail-under-lines` in `package.json`). When coverage goes up, raise the floors to match. Never lower them to make a change pass.

## Before you push or merge

There is no CI server: checks run on this Mac. Run `pnpm check` (typecheck, UI and Rust tests with coverage, then an unsigned app build) and push or merge only when it passes.
- To try a change in the desktop app, build **Moss Dev** with `pnpm app` (signed, so the Keychain doesn't ask again), quit any running Moss Dev and open `target/release/bundle/macos/Moss Dev.app`. `pnpm tauri dev` runs Moss Dev too. See "Moss and Moss Dev" below.
- Before finishing, run coverage and read the "Uncovered Line #s" for every file you touched. Cover them, or be able to say why each one can't be.
- Lines that may stay uncovered, and only these:
  - React ref guards that can't be null while mounted (`if (!el) return`).
  - `src-tauri` `run()` and `main()`, which launch the desktop window.
  - Anything that needs a real layout engine or GPU and can't be stubbed sensibly.
- Don't game the number: no tests without assertions, no excluding files to dodge it. Coverage excludes are only tests, `src/test/`, `src/main.tsx`, the demo data, and the few-line entry files in `scripts/` (their logic lives in `scripts/lib/`, which is covered).

## Moss and Moss Dev

There are two apps, and they never share anything:

- **Moss** (`app.moss.mail`) is the installed app in /Applications, which the owner uses every day as their real mail app. Only `pnpm release` makes it.
- **Moss Dev** (`app.moss.mail.dev`) is what `pnpm app` and `pnpm tauri dev` build: its own name, data folder (`~/Library/Application Support/app.moss.mail.dev`), Keychain items, agent (MCP) port and name, and a Dock icon with a DEV band. It never checks for updates. `src-tauri/tauri.dev.conf.json` lays this over `tauri.conf.json`, which stays the installed Moss's.

Test in Moss Dev and leave the installed Moss alone: don't quit, replace or relaunch it, and don't read or change `~/Library/Application Support/app.moss.mail` or its Keychain items. Moss Dev starts empty. To try it on a copy of your mail, see the README's "Build it yourself". The rules are in `src-tauri/src/identity.rs` and `scripts/lib/identity.ts`; a release refuses any config or build that isn't the installed Moss's.

## When you're done with a worktree

A worktree with its Rust `target/` folder takes 6–10 GB. Once its work is merged into main, remove it straight away:

1. Check it is finished: `git merge-base --is-ancestor <worktree HEAD> main` succeeds and `git -C <worktree> status --porcelain` prints nothing.
2. `git worktree remove <path>` (no `--force`: if it refuses, something is unsaved, so look before you lose it), then `git worktree prune`.
3. Keep the branch, so the work can still be found.

The same goes for throwaway worktrees you make yourself, e.g. a clean copy to run `pnpm check` or build the app from.

## Words people read

- No em dashes in anything public or user-facing: the UI, demo data, README, docs, release notes, GitHub text. Use a full stop, a comma, a colon or brackets.

## Releasing

`pnpm set-version` and `pnpm release`: see [docs/releasing.md](docs/releasing.md). `pnpm app` builds Moss Dev, which has no updater and no update archive, so it doesn't need the update key.

## Writing tests

- UI: Vitest + Testing Library. Tests sit next to the code (`Foo.tsx` → `Foo.test.tsx`, `store.ts` → `store.test.ts`).
- Build data with `src/test/fixtures.ts` (`thread()`, `message()`, `person()`, `resetStore()`), not `src/data/demo.ts`, so demo edits can't break tests. Screens and `App` read the demo data themselves, so their tests use `resetDemoStore()`.
- Assert on what people see and do: text, roles, `aria-*`, callbacks, store state. Not class names, not snapshots.
- Button names run label and key together with no space: "ReplyR", "SendCtrl+↵". jsdom isn't a Mac, so ⌘ shows as "Ctrl+".
- jsdom has no layout. Browser APIs the app needs (ResizeObserver, scrollTo, scrollIntoView, Range rects…) are stubbed in `src/test/setup.ts`; add new ones there, not in each test.
- `src/App.test.tsx` renders every `?scene=` state. Add a new scene to `src/dev/scenes.ts` (or the feature’s own `*Scenes.ts` beside it, e.g. `addAccountScenes.ts`) and it gets a test row there too.
- Rust: unit tests in `#[cfg(test)] mod tests` at the bottom of the file; tests that cross crates go in that crate's `tests/`.
- Swift (`apple/`, the Apple Intelligence helper): Swift Testing in `apple/Tests/`. Its logic lives in `MossAppleCore`, where the tests reach it; `Sources/moss-apple/main.swift` only listens and passes bytes along. Tests that run Apple's model are skipped on a Mac without Apple Intelligence.
- A model with a smaller context than the Gemma writer's (Apple Intelligence: 8k) gets every text budget scaled with `chat::room`; check budget changes against both.
- Several agents running coverage at once overwrite each other's `coverage/` folder. Give each its own: `pnpm vitest run --coverage --coverage.reportsDirectory=<own dir> <paths>`.
