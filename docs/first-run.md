# First run: connect Gmail

Moss signs in to Gmail with an **app password**. You don't need a Google Cloud project.
It takes about 2 minutes.

## 1. Turn on 2-Step Verification

Google only makes app passwords when 2-Step Verification is on.

1. Open <https://myaccount.google.com/signinoptions/twosv>.
2. If it says **Off**, turn it on and follow the steps.

## 2. Make an app password

1. Open <https://myaccount.google.com/apppasswords>.
2. Type a name, for example `Moss`.
3. Click **Create**.
4. Google shows 16 letters in 4 groups, like `abcd efgh ijkl mnop`. Copy them.

You can paste them with or without the spaces.

**No App passwords page?** Your Google account belongs to a school or company, and the
admin turned app passwords off. Use another account for now.

## 3. Connect in Moss

1. Start Moss. With no account yet, it opens on **Add account**.
2. Press `1` (or ↵) for **Gmail**.
3. Type your Gmail address.
4. Paste the app password (`⌘V`).
5. Press ↵ (**Connect**).

Moss checks the password with Gmail first. Nothing is saved unless it works.
The password goes to the macOS Keychain, never to a file.

## 4. While it syncs

- Your newest mail comes first. You can start reading straight away: press ↵ (**Go to inbox**).
- Older mail keeps downloading in the background.
- New mail arrives by itself (IMAP IDLE). No need to refresh.

## What each key does to Gmail

| You do in Moss | What happens in Gmail |
|---|---|
| Open an email | Marked as read |
| `E` Done | Archived (it leaves the Inbox, stays in All Mail) |
| `H` Remind me | Nothing in Gmail. Moss hides it until the time you pick (Later today, Tomorrow 8 AM, This weekend, Next week, or your own), on this Mac only |
| Reply, reply all, forward, send | Sent from your Gmail. Gmail files it in Sent |
| `⌘⇧↵` Send later | Nothing in Gmail until the time you pick. The email waits on this Mac and goes then while Moss is open; if Moss was closed, it goes soon after Moss opens |
| Search | Searches the mail on this Mac. Nothing goes to Google |

## If something goes wrong

- **"Google didn’t accept that app password"**: make a new one (step 2) and paste it again.
- **"Moss couldn’t reach imap.gmail.com"**: check your internet connection and try again.
- **IMAP turned off**: Gmail accounts made recently have IMAP on. If yours is off, turn it on in
  Gmail → Settings (gear) → See all settings → Forwarding and POP/IMAP → Enable IMAP.

Server settings, in case you need them: IMAP `imap.gmail.com` port 993 (SSL),
SMTP `smtp.gmail.com` port 465 (SSL).

## Remove the account

Settings (`⌘,`) → Accounts → **Remove account…**. Moss deletes the mail it downloaded and the
password in the Keychain. Nothing changes in Gmail. To stop the password working everywhere,
delete it at <https://myaccount.google.com/apppasswords>.

Moss won't remove an account while it still has scheduled mail, an unsent email, or an email it
couldn't confirm was sent. It tells you which. Changes still waiting to reach Gmail don't stop it:
Moss asks first, because they will be lost. See [account-removal.md](account-removal.md).
