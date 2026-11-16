# Where secrets are kept

Secrets are passwords, OAuth tokens, API keys and the MCP token. They never go into the mail database or logs.

## Release builds on macOS

- Secrets live in the macOS Keychain.
- If the Keychain fails, Moss says so. It never falls back to a file.
- Each sign-in gets its own Keychain item, with a random part in its name. Signing in again makes a new item, so an old cleanup can't delete it. See [account-removal.md](account-removal.md).

## Debug builds, and Linux without a Secret Service

These use an encrypted file in the data folder instead.

- macOS ties Keychain access to the exact binary. A debug build changes every time you rebuild, so the Keychain would ask for your password each time.
- Secrets are sealed with XChaCha20-Poly1305. The key is a separate file beside them.
- Anyone who can read both files can read the secrets. This guards against a stray copy or a casual search, not against someone with access to your user account.
- Both files are set to mode `0600` when opened, including copies restored with wider permissions. If that fails, the store doesn't open.
- If the secrets file is there but its key is missing, Moss refuses to open it and does not make a new key. Put the matching key back to recover.

How writes work:

- A write goes to a private temporary file, is flushed, then replaces the old file. The folder is flushed too.
- A flush failure is reported, even if the new file is already in place. Retry rather than assume nothing changed.
- The file store needs a filesystem that supports these flushes. A network or FUSE drive that doesn't will fail with an error.
- A lock file keeps two Moss processes from writing at once. Don't delete it while Moss runs.

## Moving from a debug build to a release

Both builds share the data folder. When a release build finds a sign-in only in the debug file:

1. It copies the sign-in into the Keychain.
2. It reads it back to check the copy.
3. Only then does it remove that one entry from the file. Other entries stay.

## The data folder

- On a Mac, the data folder is `~/Library/Application Support/app.moss.mail`.
- At start, Moss creates it with mode `0700`, or tightens an existing one to `0700`, before opening secrets or mail. If that fails, Moss doesn't start.
- A symlink in place of the folder is refused.
- Only the folder's own mode changes. Files inside, and parent folders, are left alone.
- In debug builds, `MOSS_DATA_DIR` points Moss at another folder, for tests. Give it a folder of its own, not a shared one.
