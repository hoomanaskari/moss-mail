"""Seeds a local GreenMail user for tests/greenmail_size.rs: a typical mailbox of 1,000 sent
emails and 20 in the Inbox. Text is made-up words, HTML is newsletter- or reply-style, and
attachment bytes are random, so they don't compress (like real PDFs and photos).

    python3 -I crates/moss-engine/tests/greenmail_size_seed.py /path/to/ca.der

Prints how big the mailbox is on the server.
"""
import base64
import imaplib
import random
import ssl
import sys
from datetime import datetime, timedelta, timezone
from email.utils import format_datetime

CA = sys.argv[1]
ctx = ssl.create_default_context(cafile=None)
ctx.load_verify_locations(cadata=open(CA, "rb").read())
imap = imaplib.IMAP4_SSL("127.0.0.1", 3993, ssl_context=ctx)
imap.login("sam@moss.test", "secret")
imap.create("Sent")

rng = random.Random(7)
now = datetime.now(timezone.utc)
SYLLABLES = ["ka", "lo", "mer", "ti", "sun", "va", "dre", "po", "nix", "ul"]
PEOPLE = [f"Person {n} <person{n}@example.org>" for n in range(60)]
total = 0


def word():
    # Common words far more often than rare ones, from a 5,000-word vocabulary.
    i = int((rng.random() ** 3) * 5000) + 1
    w = ""
    while i:
        w += SYLLABLES[i % 10]
        i //= 10
    return w


def prose(n):
    out = []
    size = 0
    while size < n:
        w = word()
        out.append(w + ("\n" if rng.random() < 0.08 else " "))
        size += len(w) + 1
    return "".join(out).strip()


def b64(data):
    text = base64.b64encode(data).decode()
    return "\r\n".join(text[i : i + 76] for i in range(0, len(text), 76))


def html_of(text, newsletter=False):
    style = "font-family:Helvetica,Arial,sans-serif;font-size:15px;line-height:22px;color:#333333"
    rows = "".join(f'<tr><td style="{style};padding:8px 24px">{line}</td></tr>\r\n' for line in text.split("\n"))
    if newsletter:
        rows = rows * 3 + '<tr><td><img src="cid:logo@x" alt="logo"></td></tr>'
    return f'<!doctype html><html><body style="margin:0"><table width="100%">{rows}</table></body></html>'


def append(box, date, headers, parts):
    """parts: list of (content type, extra headers, body text already encoded)."""
    global total
    boundary = f"b{rng.getrandbits(48):x}"
    lines = [*headers, f"Date: {format_datetime(date)}", "MIME-Version: 1.0", f'Content-Type: multipart/mixed; boundary="{boundary}"', ""]
    for ctype, extra, body in parts:
        lines += [f"--{boundary}", f"Content-Type: {ctype}", *extra, "", body]
    lines.append(f"--{boundary}--")
    raw = "\r\n".join(lines).encode()
    total += len(raw)
    status = imap.append(box, "\\Seen" if box == "Sent" else None, imaplib.Time2Internaldate(date.timestamp()), raw)[0]
    assert status == "OK", status


def alternative(text, html):
    inner = f"a{rng.getrandbits(48):x}"
    body = "\r\n".join(
        [f"--{inner}", "Content-Type: text/plain; charset=utf-8", "", text, f"--{inner}", "Content-Type: text/html; charset=utf-8", "", html, f"--{inner}--"]
    )
    return (f'multipart/alternative; boundary="{inner}"', [], body)


def file(name, ctype, size, inline_id=None):
    data = rng.randbytes(size)
    extra = ["Content-Transfer-Encoding: base64"]
    if inline_id:
        extra += [f"Content-ID: <{inline_id}>", f'Content-Disposition: inline; filename="{name}"']
    else:
        extra += [f'Content-Disposition: attachment; filename="{name}"']
    return (f'{ctype}; name="{name}"', extra, b64(data))


# Sent: 1,000 emails over two years. Replies quote the email before; some carry files.
for n in range(1000):
    date = now - timedelta(minutes=n * 1051 + rng.randrange(600))
    to = rng.choice(PEOPLE)
    reply = rng.random() < 0.6
    new = prose(rng.randrange(300, 1800))
    quoted = "\n".join("> " + line for line in prose(rng.randrange(800, 3000)).split("\n")) if reply else ""
    text = f"{new}\n\nSam\n\nOn Monday, {to} wrote:\n{quoted}" if reply else f"{new}\n\nSam"
    html = html_of(new) + (f"<blockquote>{html_of(quoted)}</blockquote>" if reply else "")
    parts = [alternative(text, html)]
    kind = rng.random()
    if kind < 0.10:
        parts.append(file(f"document-{n}.pdf", "application/pdf", rng.randrange(100_000, 800_000)))
    elif kind < 0.15:
        parts.append(file(f"photo-{n}.jpg", "image/jpeg", rng.randrange(1_000_000, 3_000_000)))
    if rng.random() < 0.15:
        parts.append(file("signature.png", "image/png", 8_000, inline_id=f"sig{n}@x"))
    subject = ("Re: " if reply else "") + prose(30).replace("\n", " ")[:40]
    append("Sent", date, [f"From: Sam Lee <sam@moss.test>", f"To: {to}", f"Subject: {subject}", f"Message-ID: <sent{n}@moss.test>"], parts)

# Inbox: 20 emails from the last two weeks: newsletters, receipts, conversations, documents.
for n in range(20):
    date = now - timedelta(hours=n * 15 + 1)
    sender = rng.choice(PEOPLE)
    kind = ["newsletter"] * 6 + ["receipt"] * 5 + ["conversation"] * 7 + ["document"] * 2
    kind = kind[n]
    headers = [f"From: {sender}", "To: Sam Lee <sam@moss.test>", f"Subject: {kind} {n}", f"Message-ID: <inbox{n}@example.org>"]
    if kind == "newsletter":
        text = prose(6000)
        parts = [alternative(text, html_of(text, newsletter=True)), file("logo.png", "image/png", 8_000, inline_id="logo@x")]
        headers.append("List-Id: <news.example.org>")
    elif kind == "receipt":
        text = prose(1500)
        parts = [alternative(text, html_of(text))]
        if n % 2:
            parts.append(file(f"receipt-{n}.pdf", "application/pdf", 80_000))
    elif kind == "conversation":
        text = prose(rng.randrange(400, 2000))
        parts = [alternative(text, html_of(text))]
        if n % 3 == 0:
            parts.append(file("signature.png", "image/png", 15_000, inline_id=f"sig-in{n}@x"))
    else:
        text = prose(800)
        parts = [alternative(text, html_of(text)), file(f"contract-{n}.pdf", "application/pdf", 600_000)]
    append("INBOX", date, headers, parts)

print(f"1,020 emails, {total / 1e6:.1f} MB on the server ({total / 1020 / 1024:.1f} KB an email)")
