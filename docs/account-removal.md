# Removing an account

What happens when you remove an account in Settings, Accounts.

## When Moss refuses

Moss checks first, before it stops anything or touches the Keychain. It refuses while the account has:

- a send that isn't finished (`held`, `claimed`, `accepted` or `uncertain`) and hasn't been reviewed in Drafts,
- scheduled mail.

The message says which one. Sort it out, then remove the account again. The same checks run again inside the delete itself.

## Server changes that haven't gone yet

Server changes still in the queue don't stop the removal on their own. Moss asks first: "N changes haven't reached the server and will be lost." If you say yes, they are deleted in the same transaction as the account. If the number changed since you were asked, nothing is removed.

Moss also refuses if the account's saved connection is missing, unreadable, or names a different address. Without it, Moss can't be sure which Keychain item belongs to the account.

## What gets removed

In one database transaction:

- the account, its mail, drafts and settings,
- its saved connection,
- a note of the exact Keychain item to remove.

Finished sends (`complete` and `cancelled`) are kept as small records without their content. They stop an old operation from being sent again if you add the account back.

Only after that commit does Moss stop the account's sync worker and remove its Keychain item. If the transaction fails, the account, its sign-in and its worker stay as they were.

Moss also deletes the copy of the database from before the last update (`moss.db.before-update`), because it still holds this account's mail. If that fails, the account is still removed.

Nothing changes on the server. For Resend, Moss's copy may be the only one left, so Settings asks you to confirm you are deleting it for good.

## Removing the Keychain item

- Moss removes only the exact item it saved for that account. It never guesses item names or matches by email address.
- It skips an item that another saved connection still uses.
- It counts an item as gone only after reading back that it is absent.
- If removal fails, the note stays. Moss tries again at the next start, and the app shows a notice with a Retry button.
- Each try handles at most 128 items, taking turns, so one stuck item can't block the rest.

## Signing in again

Each new sign-in saves its secret under a new Keychain item with a random part in its name. Signing in again never overwrites an older item, so a late cleanup can't delete the new sign-in.

- The old item is noted for the same cleanup, in the same transaction that saves the new connection.
- If saving the new connection fails, the new item is noted for cleanup instead. Cleanup still skips anything a saved connection uses.
