"""Seeds the local GreenMail server for tests/greenmail.rs. Each test has its own mailbox,
so what one test sends, files or deletes never changes what another one sees:

- maya@moss.test (sign in, sync, reply): a small inbox, a big message with an inline logo
  and a PDF, and more Sent mail than Moss keeps whole.
- delete@moss.test: two emails to delete and bring back.
- important@moss.test: an invoice to move to Important and out.
- sendlater@moss.test, drafts@moss.test: folders only.
- daniel@moss.test: nothing; the others write to him and he writes to them.

    python3 -I crates/moss-engine/tests/greenmail_seed.py /path/to/ca.der
"""
import base64
import imaplib
import os
import ssl
import sys
from datetime import datetime, timedelta, timezone
from email.utils import format_datetime, make_msgid

CA = sys.argv[1]
ctx = ssl.create_default_context(cafile=None)
ctx.load_verify_locations(cadata=open(CA, "rb").read())

# MOSS_E2E_PORT_OFFSET moves the ports, as in the Rust tests, so several servers can run at once.
OFFSET = int(os.environ.get("MOSS_E2E_PORT_OFFSET", "0"))
imap = None
TO = None


def sign_in(email, name):
    """Switches to `email`'s mailbox and gives it the folders Moss files mail into."""
    global imap, TO
    if imap:
        imap.logout()
    imap = imaplib.IMAP4_SSL("127.0.0.1", 3993 + OFFSET, ssl_context=ctx)
    imap.login(email, "secret")
    TO = f"{name} <{email}>"
    for box in ("Archive", "Sent", "Trash"):
        print(email, box, imap.create(box)[0])


now = datetime.now(timezone.utc)

# Moss keeps the newest 1,000 sent messages whole (SENT_KEEP); the rest come headers only.
SENT = 1003


def pattern(n, step):
    """Attachment bytes the Rust test can work out again: (i * step + 7) % 256."""
    return bytes((i * step + 7) % 256 for i in range(n))


def b64(data):
    text = base64.b64encode(data).decode()
    return "\r\n".join(text[i : i + 76] for i in range(0, len(text), 76))


def append(box, date, lines):
    raw = "\r\n".join(lines).encode()
    return imap.append(box, None, imaplib.Time2Internaldate(date.timestamp()), raw)[0]


def head(subject, date, frm, to, headers=()):
    return [
        f"From: {frm}",
        f"To: {to}",
        f"Subject: {subject}",
        f"Message-ID: {make_msgid(domain='moss.test')}",
        f"Date: {format_datetime(date)}",
        "MIME-Version: 1.0",
        *headers,
    ]


def mail(subject, body, minutes_ago, frm="Daniel Okafor <daniel@moss.test>", headers=(), html=None):
    date = now - timedelta(minutes=minutes_ago)
    lines = head(subject, date, frm, TO, headers)
    if html:
        lines += [
            'Content-Type: multipart/alternative; boundary="b"',
            "",
            "--b",
            "Content-Type: text/plain; charset=utf-8",
            "",
            body,
            "--b",
            "Content-Type: text/html; charset=utf-8",
            "",
            html,
            "--b--",
        ]
    else:
        lines += ["Content-Type: text/plain; charset=utf-8", "", body]
    print(subject, append("INBOX", date, lines))


sign_in("maya@moss.test", "Maya Ross")
mail("Q4 design budget", "Hi Maya,\n\nCan you sign off the Q4 design budget by Friday?\n\nThanks,\nDaniel", 30)
mail("Venue options for the offsite", "Hi Maya,\n\nThree venues are free in March. Which do you like?\n\nDaniel", 300)
mail(
    "The weekly design digest",
    "Five links worth reading this week.",
    600,
    frm="Design Digest <news@digest.moss.test>",
    headers=("List-Unsubscribe: <mailto:unsubscribe@digest.moss.test>",),
    html='<h1>Design digest</h1><p>Five links worth reading.</p><img src="https://example.com/pixel.png" width="1" height="1">',
)
mail("Your receipt from Framely", "Thanks for your payment of $15.", 900, frm="Framely <billing@framely.moss.test>")

# Bigger than Moss fetches whole: sync leaves the PDF on the server, keeps the logo.
date = now - timedelta(minutes=60)
deck = head("Q4 deck", date, "Daniel Okafor <daniel@moss.test>", "Maya Ross <maya@moss.test>") + [
    'Content-Type: multipart/mixed; boundary="mix"',
    "",
    "--mix",
    'Content-Type: multipart/related; boundary="rel"',
    "",
    "--rel",
    'Content-Type: multipart/alternative; boundary="alt"',
    "",
    "--alt",
    "Content-Type: text/plain; charset=utf-8",
    "",
    "The Q4 deck is attached.",
    "--alt",
    "Content-Type: text/html; charset=utf-8",
    "",
    '<p>The Q4 deck is attached.</p><img src="cid:logo@moss.test">',
    "--alt--",
    "--rel",
    "Content-Type: image/png",
    "Content-ID: <logo@moss.test>",
    'Content-Disposition: inline; filename="logo.png"',
    "Content-Transfer-Encoding: base64",
    "",
    b64(pattern(3000, 13)),
    "--rel--",
    "--mix",
    'Content-Type: application/pdf; name="Q4.pdf"',
    'Content-Disposition: attachment; filename="Q4.pdf"',
    "Content-Transfer-Encoding: base64",
    "",
    b64(pattern(300_000, 31)),
    "--mix--",
]
print("Q4 deck", append("INBOX", date, deck))

# Oldest first, so the lowest UIDs are the ones that come headers only.
for i in range(SENT):
    date = now - timedelta(hours=SENT - i)
    lines = head(f"Sent note {i}", date, "Maya Ross <maya@moss.test>", "Daniel Okafor <daniel@moss.test>")
    lines += ["Content-Type: text/plain; charset=utf-8", "", f"Note number {i}."]
    append("Sent", date, lines)
print("Sent", SENT)

# For delete and undo: one stays in the inbox, the other is marked done first.
sign_in("delete@moss.test", "Dee Lee")
mail("Delete me from the inbox", "This one goes to Trash and back.", 1200)
mail("Delete me once done", "This one is done, then goes to Trash and back to the archive.", 1300)

# For moving to Important and out, starred and unstarred here and on the server.
sign_in("important@moss.test", "Imani Port")
mail("Your invoice from Linear", "Your invoice for November is ready.", 1400, frm="Linear <billing@linear.moss.test>")

sign_in("sendlater@moss.test", "Sam Later")
sign_in("drafts@moss.test", "Dara Fitz")
imap.logout()
