# Lean storage: what Moss keeps, measured

Moss keeps what it needs to list, find and sort all your mail, plus the text of your mail
(compressed). Files stay on the server and come when you open them.

## Measured

**End to end on GreenMail** (`crates/moss-engine/tests/greenmail_size.rs`, seed
`greenmail_size_seed.py`): 1,000 sent emails and 20 in the Inbox. Made-up words; newsletter
and reply-style HTML; random, incompressible attachments (PDFs, photos, signature images).

| | |
|---|---|
| On the server | 204 MB (195 KB an email) |
| Sync downloaded | 10.5 MB (10.1 KB an email) |
| Index & summaries | 2.7 MB (2.6 KB an email) |
| Offline email (all text, zstd) | 2.0 MB (1.9 KB an email) |
| File bytes | none |
| Database file | **4.6 MB (4.4 KB an email)** |
| After opening every inbox email | + 48 KB of cache (the images they show) |

**Store fixture** (`crates/moss-store/tests/storage_size.rs`, 1,000 typical messages, 296 KB
each as sent): 6.4 KB a message: index 2.8 KB, compressed text 3.5 KB. Before the storage
branch: 332 KB; with it: 39 KB.

Real mail will be bigger: real HTML compresses less than the made-up kind. The design's
estimate of about 5 KB of text an email still holds as a guide.

What it was on a real Gmail account before: 2,665 emails took 1,176 MB, 94% of it originals.

## What's where

- `messages`: headers, 150-character preview, state. `details`: amounts, dates, tracking and
  order numbers. `people`: counts from headers. `embeddings`: 256 bytes an email.
  `summaries`: a line for conversations that matter. `message_search`: FTS5 with no content.
- `bodies`: text and HTML, zstd, `method` recorded; `offline` (kept by the setting) or
  cache (opened, removed oldest first past the limit, 500 MB by default).
- `attachment_data`: file bytes, as cache (opened) or offline (with "Everything").
- `attachment_text`: the text inside files, read on this Mac (`moss-extract`: PDFKit, Vision
  OCR for scans and photos, Office XML, plain text), zstd like bodies. One row per file read,
  even with no words, so each file is read once. Indexed by `attachment_search` (FTS5, no
  content) and `attachment_embeddings`. Read in the background for the last 30 days of mail
  from people (Important and Other, not on a low battery or a hotspot), the rest when Ask Moss
  needs it. Settings → Storage → Search inside attachments; off forgets it all.
- `message_raw`: only where no server keeps the mail (Resend, mail sent from here until the
  server's copy arrives).

## Sync

- No attachments. History from All Mail only for conversations in the Inbox, as headers.
- Newest first, Inbox first, resumable; stops at 2.4 GB a day (Gmail's IMAP limit is 2.5 GB).
