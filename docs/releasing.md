# Releasing Moss

How a new version of Moss gets from this Mac to everyone's. Moss is released for macOS on Apple silicon, from an Apple silicon Mac. The bundle requires macOS 13 or later because native OCR uses Vision automatic language detection. Apple Intelligence has its own newer system requirements.

## Once, before the first release

You need four things on the Mac that makes releases.

1. **The Developer ID certificate** for Pixel Forty Inc. (team `2SQ9JNU8XE`) in the Keychain. Check with:

   ```bash
   security find-identity -v -p codesigning
   ```

   One line should say `Developer ID Application: Pixel Forty Inc.`

2. **A notary login**, saved in the Keychain as `moss-notary`. Apple checks every release with it.
   1. At [account.apple.com](https://account.apple.com), open Sign-In and Security, then App-Specific Passwords, and make one called "Moss notarize".
   2. Save it (Terminal asks for the password; paste the app-specific one):

      ```bash
      xcrun notarytool store-credentials moss-notary --apple-id YOUR_APPLE_ID --team-id 2SQ9JNU8XE
      ```

3. **The update key** at `~/.tauri/moss-updater.key`. Every Moss checks that an update was signed with it, so:
   - **Back it up** (in 1Password, for example). If it's lost, people can't get updates and have to download Moss again by hand.
   - **Never share it** or commit it. Its public half is in `src-tauri/tauri.conf.json`.

4. **The tools:** Node 24, the package-manager version in `package.json`, `brew install create-dmg minisign`, and the GitHub CLI signed in (`gh auth login`).

## Each release

1. **Pick the version** and set it everywhere it's kept (package.json, the Tauri config, the Rust workspace and Cargo.lock):

   ```bash
   pnpm set-version 0.2.0
   ```

2. **Write what's new** in `docs/releases/0.2.0.md`, for people, not developers: a line or two, then a short list. No em dashes. It goes on the GitHub Release and into the update itself.

3. **Check and commit:**

   ```bash
   pnpm check
   git commit -am "Moss 0.2.0"
   git push
   ```

   Also check the dependencies for known problems with `pnpm audit` and `cargo audit` (`cargo install cargo-audit`). Read the warnings as well as the count. A warning for a package that only Linux or Windows builds use still matters before a release for those systems.

4. **Make the release:**

   ```bash
   pnpm release
   ```

   It refuses a dirty tree, a branch other than main, an upstream commit mismatch, or an existing version tag or release. Then it:
   - captures the exact main commit into a fresh temporary source directory,
   - records the source hash, commit and toolchain versions,
   - installs dependencies with the frozen pnpm lockfile and runs `pnpm check` in that directory,
   - verifies that source files and provenance have not changed,
   - builds Moss.app from that same directory, embeds `release-source.json` and signs it with the Developer ID,
   - sends it to Apple to notarize, and staples the ticket,
   - makes the update archive and signs it with the update key,
   - makes the DMG, signs it, notarizes and staples it,
   - writes `latest.json`, which every Moss reads to find the update,
   - checks source identity again, verifies the update signature against the app's public key and signed version, and records hashes of the four artifacts,
   - copies and verifies the artifacts in a unique production output directory,
   - drafts a GitHub Release `v0.2.0` against the captured commit, with the four artifacts and `release-manifest.json`.

   The temporary source and build directory is removed on success or failure. Completed output files are kept in `target/release/moss-release/production/<version>/<source-hash>-<unique-id>/`. The command prints that path. Signing or notarization can still fail after checks pass; a failed phase does not create a GitHub draft.

5. **Try the DMG** from the draft on a Mac. Before publishing, compare the draft's target commit with `source.origin.sha` in `release-manifest.json`. If the version tag already exists, verify that it resolves to that same commit. Do not publish if they differ. The command checks tag availability before the build and again before uploading, but cannot prevent somebody changing a tag later. Then **publish** the draft on GitHub. Publishing is what makes it the update: Moss reads `latest.json` from the newest published release.

## Trying it without releasing

```bash
pnpm release --local
```

This captures current working files, including uncommitted changes, and runs the same isolated validation and build. It signs locally, without notarizing or uploading a release. Dependency installation may download packages. Rehearsal artifacts have working-copy provenance and land in `target/release/moss-release/local/<version>/<source-hash>-<unique-id>/`. The production publishing path refuses working-copy provenance.

Use `pnpm release`, not the internal `scripts/package-release.sh`, to run the complete gate. An isolated build requires extra disk space for dependencies and Rust build output. It does not reuse another checkout's writable build directory.

## What the release record proves

`release-manifest.json` records source file hashes, executable modes, the commit or local-rehearsal origin, toolchain versions and artifact hashes. The app contains the source and toolchain record as `release-source.json`, covered by its code signature. The public manifest contains no environment values or signing credentials.

The gate checks source identity at phase boundaries. It detects lasting edits, added or removed source files and symlink replacements. It does not sandbox trusted build tools or detect a build step that changes a source file and restores it within one phase. Dependency lockfiles, reviewed build scripts and a trusted build Mac remain part of the release process. Installation and checks do not receive GitHub tokens, the SSH agent socket or signing settings. Packaging receives signing settings, but no GitHub token or SSH agent socket. This environment separation does not prevent same-user file access to credentials saved on the build Mac.

Use the standard Node, pnpm and Rust toolchain locations on the release Mac. Build environment overrides, including custom Cargo, Rustup or pnpm home variables, are not forwarded.

## If something goes wrong

- **A partial GitHub upload fails:** inspect the draft and its assets. After confirming it is incomplete and unpublished, delete that draft manually before rerunning. The command does not delete remote releases automatically.
- **Interrupted or incomplete cleanup:** temporary snapshots are named `moss-release-*` under the system temporary directory. If the command reports a cleanup failure, or is killed before cleanup runs, remove only its reported directory after confirming no release process is using it. A cleanup failure does not change the reported result of draft creation.
- **Notarization fails:** `xcrun notarytool log <submission id> --keychain-profile moss-notary` says why.
- **A bad release is out:** publish a fixed version with a higher number. Moss only moves forward, so deleting a release doesn't take it back from people who already have it.
