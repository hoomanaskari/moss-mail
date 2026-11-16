# Backups

Moss can make a backup of its mail database. It can't restore one yet.

## Making a backup

Settings, Storage. Pick where to save it. Moss shows progress, lets you cancel, and shows the file's SHA-256 when it's done.

- One backup runs at a time. It keeps going if you leave the Settings screen.
- Quitting Moss cancels a running backup and waits up to 30 seconds for it to stop.
- Moss picks the destination with its own save dialog. The window can't name a path.

## What's in it

- Your local mail, Resend originals, drafts and their files, and send records.
- Not your settings, sign-ins or connection details.
- Not the list of Keychain items waiting to be removed.
- Not mail that lives only on the server. Files Moss never downloaded aren't in it.

A backup still holds your mail. Treat it as private.

## How it's made

1. Moss copies the database with SQLite's backup API, from one consistent snapshot. Changes made during the copy aren't mixed in.
2. The copy is made in a private folder beside the database, not in the destination.
3. Settings and the Keychain cleanup list are deleted, and `VACUUM` removes their old bytes.
4. Moss checks the copy's integrity and foreign keys.
5. Only the cleaned, checked file goes to the destination. Its checksum is checked after copying.
6. The final file never replaces an existing file. On macOS it has mode `0600`.
7. Moss reads the saved file back and checks it before it calls the backup done. If that fails, it says: "The backup was saved, but reading it back failed, so it can't be trusted. Delete it and make a new one."

Limits:

- A backup of more than 64 GiB is refused.
- This is a size limit, not a disk space check. Making a backup needs some free space beside the database too.

## A backup can't be opened by mistake

The backup is marked with its own format and a schema version no Moss accepts as a live database. Renaming it to `moss.db` doesn't work: Moss refuses to open it, so old sends or cleanups in it can't run.

## Safety copy before an update

This is separate from backups.

- A new Moss updates a database an older Moss left only while no other Moss has it open. Otherwise it says "Another Moss is using this mail. Quit it first." and changes nothing.
- Before the update it saves a full copy beside it: `moss.db.before-update`. A copy from an earlier update is removed first.
- If the copy can't be made (no space, say), the update goes ahead anyway.
- If an update fails, Moss's start-up message points to this copy so it can be put back by hand.
- The copy is deleted on the next launch that needs no update, and when an account is removed (it holds that account's mail).
- Work files left by a backup or update that stopped partway are cleared at launch.

## Not there yet

- Restoring a backup.
- Exporting mail to a standard format such as `.mbox`.
