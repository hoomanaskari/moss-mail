//! Versioned migrations. Each entry moves the schema up one version; `PRAGMA user_version`
//! records how far a database has got. Never edit a shipped migration: add a new one.

pub const MIGRATIONS: &[&str] = &[V1, V2, V3, V4, V5, V6, V7, V8, V9, V10, V11, V12, V13, V14, V15, V16, V17, V18, V19, V20, V21, V22, V23, V24, V25, V26, V27, V28, V29, V30, V31, V32, V33, V34, V35, V36, V37, V38, V39, V40, V41, V42, V43, V44, V45, V46, V47, V48, V49, V50, V51];

/// Accounts, mailboxes, threads, messages and their state, attachments, drafts, sync
/// checkpoints and the keyword index.
///
/// Embeddings for hybrid search come in a later migration: a `sqlite-vec` table keyed
/// by `messages.id`, filled from `body_stripped` like the keyword index is.
const V1: &str = r#"
-- No secrets here: passwords and tokens live in the OS keychain.
CREATE TABLE accounts (
    id          TEXT PRIMARY KEY,
    name        TEXT NOT NULL,
    letter      TEXT NOT NULL,
    address     TEXT NOT NULL,
    sender_name TEXT NOT NULL,
    color       TEXT NOT NULL,
    provider    TEXT NOT NULL,
    org         TEXT,
    org_domain  TEXT,
    position    INTEGER NOT NULL
);

CREATE TABLE mailboxes (
    id         INTEGER PRIMARY KEY,
    account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    name       TEXT NOT NULL,
    role       TEXT,
    UNIQUE (account_id, name)
);

-- mailbox is '' for checkpoints that cover the whole account, like a Resend list cursor.
CREATE TABLE sync_checkpoints (
    account_id     TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    mailbox        TEXT NOT NULL,
    uid_validity   INTEGER,
    uid_next       INTEGER,
    highest_modseq INTEGER,
    cursor         TEXT,
    updated_at     INTEGER NOT NULL,
    PRIMARY KEY (account_id, mailbox)
);

CREATE TABLE threads (
    id               INTEGER PRIMARY KEY,
    account_id       TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    subject          TEXT NOT NULL,
    -- Fallback grouping when headers don't link messages: see text::subject_key.
    subject_key      TEXT NOT NULL,
    participants_key TEXT NOT NULL,
    last_message_at  INTEGER NOT NULL
);
CREATE INDEX threads_by_subject ON threads (account_id, subject_key, participants_key);
CREATE INDEX threads_by_time ON threads (account_id, last_message_at DESC);

CREATE TABLE messages (
    id            INTEGER PRIMARY KEY,
    account_id    TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    thread_id     INTEGER NOT NULL REFERENCES threads(id) ON DELETE CASCADE,
    -- RFC 5322 Message-ID without <>, or "sha256:…" of the raw bytes when there is none.
    message_id    TEXT NOT NULL,
    subject       TEXT NOT NULL,
    from_name     TEXT NOT NULL,
    from_email    TEXT NOT NULL,
    to_json       TEXT NOT NULL,
    cc_json       TEXT NOT NULL,
    bcc_json      TEXT NOT NULL,
    -- Names and addresses of everyone on it, for search.
    participants  TEXT NOT NULL,
    sent_at       INTEGER NOT NULL,
    received_at   INTEGER NOT NULL,
    body_text     TEXT NOT NULL,
    body_html     TEXT,
    -- body_text without quoted replies or signature. This is what search indexes.
    body_stripped TEXT NOT NULL,
    snippet       TEXT NOT NULL,
    from_me       INTEGER NOT NULL,
    -- Per-message state. A thread's state is worked out from its messages.
    split         TEXT NOT NULL,
    section       TEXT NOT NULL,
    read          INTEGER NOT NULL DEFAULT 0,
    done          INTEGER NOT NULL DEFAULT 0,
    flagged       INTEGER NOT NULL DEFAULT 0,
    snoozed_until INTEGER,
    UNIQUE (account_id, message_id)
);
CREATE INDEX messages_by_thread ON messages (thread_id, sent_at);
CREATE INDEX messages_open ON messages (thread_id) WHERE done = 0;

-- In-Reply-To and References, for threading.
CREATE TABLE message_refs (
    message_id INTEGER NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
    ref        TEXT NOT NULL,
    PRIMARY KEY (message_id, ref)
);
CREATE INDEX message_refs_by_ref ON message_refs (ref);

CREATE TABLE message_labels (
    message_id INTEGER NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
    label      TEXT NOT NULL,
    PRIMARY KEY (message_id, label)
);

-- Where the provider has the message. One message can sit in several mailboxes.
CREATE TABLE message_locations (
    message_id  INTEGER NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
    mailbox_id  INTEGER NOT NULL REFERENCES mailboxes(id) ON DELETE CASCADE,
    provider_id TEXT NOT NULL,
    PRIMARY KEY (mailbox_id, provider_id)
);
CREATE INDEX message_locations_by_message ON message_locations (message_id);

-- The original bytes. For Resend accounts this is the only copy kept anywhere.
CREATE TABLE message_raw (
    message_id INTEGER PRIMARY KEY REFERENCES messages(id) ON DELETE CASCADE,
    raw        BLOB NOT NULL
);

CREATE TABLE attachments (
    message_id   INTEGER NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
    position     INTEGER NOT NULL,
    name         TEXT NOT NULL,
    content_type TEXT NOT NULL,
    bytes        INTEGER NOT NULL,
    content_id   TEXT,
    inline       INTEGER NOT NULL,
    PRIMARY KEY (message_id, position)
);

CREATE TABLE drafts (
    id                 TEXT PRIMARY KEY,
    account_id         TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    thread_id          INTEGER REFERENCES threads(id) ON DELETE SET NULL,
    mode               TEXT NOT NULL,
    to_json            TEXT NOT NULL,
    cc_json            TEXT NOT NULL,
    bcc_json           TEXT NOT NULL,
    subject            TEXT NOT NULL,
    body               TEXT NOT NULL,
    body_html          TEXT NOT NULL,
    remind_if_no_reply INTEGER NOT NULL,
    updated_at         INTEGER NOT NULL
);

-- Keyword search over messages, kept in step by triggers in the same transaction.
CREATE VIRTUAL TABLE message_search USING fts5 (
    subject, participants, body_stripped,
    content = 'messages', content_rowid = 'id',
    tokenize = 'unicode61 remove_diacritics 2'
);
CREATE TRIGGER messages_search_insert AFTER INSERT ON messages BEGIN
    INSERT INTO message_search (rowid, subject, participants, body_stripped)
    VALUES (new.id, new.subject, new.participants, new.body_stripped);
END;
CREATE TRIGGER messages_search_delete AFTER DELETE ON messages BEGIN
    INSERT INTO message_search (message_search, rowid, subject, participants, body_stripped)
    VALUES ('delete', old.id, old.subject, old.participants, old.body_stripped);
END;
CREATE TRIGGER messages_search_update AFTER UPDATE OF subject, participants, body_stripped ON messages BEGIN
    INSERT INTO message_search (message_search, rowid, subject, participants, body_stripped)
    VALUES ('delete', old.id, old.subject, old.participants, old.body_stripped);
    INSERT INTO message_search (rowid, subject, participants, body_stripped)
    VALUES (new.id, new.subject, new.participants, new.body_stripped);
END;
"#;

/// Small app settings, such as how each account connects. Secrets stay in the OS keychain.
const V2: &str = r#"
CREATE TABLE settings (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
"#;

/// Gmail conversations, so mail archived in Gmail (kept only in All Mail) can fill in the
/// conversations it belongs to. See `conversations.rs`.
const V3: &str = r#"
-- Gmail's conversation id (X-GM-THRID): NULL until asked, 0 when the server had none.
ALTER TABLE messages ADD COLUMN conversation_id INTEGER;
CREATE INDEX messages_by_conversation ON messages (account_id, conversation_id);

-- Conversations seen here, and whether their other messages have been fetched.
CREATE TABLE conversations (
    account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    id         INTEGER NOT NULL,
    filled     INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (account_id, id)
);
"#;

/// When each message was marked done (ms), so the Done list can show the latest first. NULL
/// while open, and for mail done before this version.
const V4: &str = r#"
ALTER TABLE messages ADD COLUMN done_at INTEGER;
"#;

/// Changes waiting to reach the server, in the order they were made: see `outbox.rs`.
const V5: &str = r#"
CREATE TABLE outbox (
    id         INTEGER PRIMARY KEY,
    account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    change     TEXT NOT NULL,
    created_at INTEGER NOT NULL
);
CREATE INDEX outbox_by_account ON outbox (account_id, id);
"#;

/// What the decision model said about each message: see `decisions.rs`. `yes` is how likely
/// the answer is yes; `acted` is whether Moss moved the message because of it.
const V6: &str = r#"
CREATE TABLE ai_decisions (
    message_id INTEGER NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
    question   TEXT NOT NULL,
    yes        REAL NOT NULL,
    acted      INTEGER NOT NULL,
    decided_at INTEGER NOT NULL,
    PRIMARY KEY (message_id, question)
);
"#;

/// Blocked and accepted senders, per account (see `senders.rs`), and which mail is blocked.
const V7: &str = r#"
-- `sender` is a lowercase address, or a domain (no @) when a whole domain is blocked.
CREATE TABLE sender_rules (
    account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    sender     TEXT NOT NULL,
    rule       TEXT NOT NULL CHECK (rule IN ('block', 'accept')),
    created_at INTEGER NOT NULL,
    PRIMARY KEY (account_id, sender)
);

-- Blocked mail stays out of the inbox and the Done list, and shows in Blocked.
ALTER TABLE messages ADD COLUMN blocked INTEGER NOT NULL DEFAULT 0;
"#;

/// Lean storage: Moss keeps what it needs to list, find and sort all your mail, and only
/// the text of the emails themselves (compressed, in their own table). Files stay on the
/// server; anything fetched to open is a cache with a size limit. See `content.rs` and
/// `bodies.rs`.
///
/// Mail from servers that keep it is cleared, so it is fetched again into the new layout
/// (people agreed to that while Moss is young). Resend mail stays, since its original is
/// the only copy: [`crate::Store::rebuild_index`] reads it back in.
const V8: &str = r#"
DROP TRIGGER messages_search_insert;
DROP TRIGGER messages_search_delete;
DROP TRIGGER messages_search_update;
DROP TABLE message_search;

DELETE FROM threads WHERE account_id IN (SELECT id FROM accounts WHERE provider <> 'resend');
DELETE FROM sync_checkpoints WHERE account_id IN (SELECT id FROM accounts WHERE provider <> 'resend');
DELETE FROM conversations WHERE account_id IN (SELECT id FROM accounts WHERE provider <> 'resend');

-- The text now lives in `bodies`, the words in the index.
ALTER TABLE messages DROP COLUMN body_text;
ALTER TABLE messages DROP COLUMN body_html;
ALTER TABLE messages DROP COLUMN body_stripped;

-- Only the headers were fetched (older mail): enough for threading and knowing you
-- replied, but there's no preview and its text isn't indexed until someone opens it.
ALTER TABLE messages ADD COLUMN headers_only INTEGER NOT NULL DEFAULT 0;

-- Where the attachment is in its message, as an IMAP body section ("2", "1.2"), to fetch
-- its bytes from the server.
ALTER TABLE attachments ADD COLUMN part TEXT;

-- Email bodies, compressed with `method`, apart from messages so lists never read them.
-- `offline` rows are kept by the offline setting; the rest are a cache of mail opened,
-- removed oldest `used_at` first once the cache is over its limit.
CREATE TABLE bodies (
    message_id INTEGER PRIMARY KEY REFERENCES messages(id) ON DELETE CASCADE,
    method     TEXT NOT NULL,
    text       BLOB NOT NULL,
    html       BLOB,
    bytes      INTEGER NOT NULL,
    offline    INTEGER NOT NULL,
    used_at    INTEGER NOT NULL
);
CREATE INDEX bodies_by_use ON bodies (offline, used_at);

-- The bytes of files: inline images and attachments opened (cache), or every file when
-- the offline setting keeps everything.
CREATE TABLE attachment_data (
    message_id INTEGER NOT NULL,
    position   INTEGER NOT NULL,
    data       BLOB NOT NULL,
    offline    INTEGER NOT NULL,
    used_at    INTEGER NOT NULL,
    PRIMARY KEY (message_id, position),
    FOREIGN KEY (message_id, position) REFERENCES attachments (message_id, position) ON DELETE CASCADE
);
CREATE INDEX attachment_data_by_use ON attachment_data (offline, used_at);

-- Keyword search that keeps no readable text: only which words are in which message.
-- Filled from the new text of each message when it arrives, before it is compressed.
CREATE VIRTUAL TABLE message_search USING fts5 (
    subject, participants, body,
    content = '', contentless_delete = 1,
    tokenize = 'unicode61 remove_diacritics 2'
);
CREATE TRIGGER messages_search_delete AFTER DELETE ON messages BEGIN
    DELETE FROM message_search WHERE rowid = old.id;
END;

-- What each message means, as a vector from the embedding model (see `meaning.rs`).
-- `model` says which model made it, so a new model rebuilds them.
CREATE TABLE embeddings (
    message_id INTEGER PRIMARY KEY REFERENCES messages(id) ON DELETE CASCADE,
    model      TEXT NOT NULL,
    vector     BLOB NOT NULL
);
CREATE INDEX embeddings_by_model ON embeddings (model);

-- Details found by simple rules: amounts, dates, tracking and order numbers.
CREATE TABLE details (
    message_id INTEGER NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
    kind       TEXT NOT NULL,
    value      TEXT NOT NULL,
    PRIMARY KEY (message_id, kind, value)
);

-- Who you hear from and write to, counted from headers.
CREATE TABLE people (
    account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    email      TEXT NOT NULL,
    name       TEXT NOT NULL,
    from_them  INTEGER NOT NULL DEFAULT 0,
    to_them    INTEGER NOT NULL DEFAULT 0,
    last_at    INTEGER NOT NULL,
    PRIMARY KEY (account_id, email)
);

-- One or two lines on what a conversation is about, from the writing model, for the
-- conversations that matter. `through` is the newest message it read.
CREATE TABLE summaries (
    thread_id INTEGER PRIMARY KEY REFERENCES threads(id) ON DELETE CASCADE,
    text      TEXT NOT NULL,
    model     TEXT NOT NULL,
    through   INTEGER NOT NULL,
    made_at   INTEGER NOT NULL
);
"#;

/// Your signatures and which one each account adds: see `signatures.rs`. Whether Moss may
/// choose is the `signatures:let_moss_choose` setting.
const V9: &str = r#"
-- `used` orders the composer's picker: higher was used more recently, NULL never.
CREATE TABLE signatures (
    id         TEXT PRIMARY KEY,
    name       TEXT NOT NULL,
    lines_json TEXT NOT NULL,
    position   INTEGER NOT NULL,
    used       INTEGER
);

CREATE TABLE account_signatures (
    account_id TEXT PRIMARY KEY REFERENCES accounts(id) ON DELETE CASCADE,
    new_emails TEXT REFERENCES signatures(id) ON DELETE SET NULL,
    replies    TEXT REFERENCES signatures(id) ON DELETE SET NULL
);
"#;

/// Deleted mail: moved to Trash on the server and out of every list here, until undone.
const V10: &str = r#"
-- Where a deleted message was when you deleted it, so undo puts it back there:
-- 'inbox' or 'done'. NULL for mail that isn't deleted.
ALTER TABLE messages ADD COLUMN deleted_from TEXT CHECK (deleted_from IN ('inbox', 'done'));
"#;

/// Nothing changes shape: opening a database from before this version rebuilds its previews
/// (see `Store::init`), so they leave out the tags, links and padding HTML mail left in them.
const V11: &str = "";

/// As V11: previews are made again, now without divider lines either.
const V12: &str = "";

/// Drafts keep their signature, so one you come back to is signed as you left it.
const V13: &str = r#"
ALTER TABLE drafts ADD COLUMN signature_id TEXT;
ALTER TABLE drafts ADD COLUMN signature_chosen INTEGER NOT NULL DEFAULT 0;
"#;

/// Forwarded mail: marked on the server by any mail app, and the forwards sent from Moss.
const V14: &str = r#"
-- IMAP $Forwarded: set by Moss or another mail app.
ALTER TABLE messages ADD COLUMN forwarded INTEGER NOT NULL DEFAULT 0;

-- A message you forwarded from Moss, and the sent copy that carried it.
CREATE TABLE forwards (
    message_id INTEGER NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
    sent_id    INTEGER NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
    PRIMARY KEY (message_id, sent_id)
);
"#;

/// The other addresses each account sends from (see `addresses.rs`), and which of them a
/// draft goes from.
const V15: &str = r#"
-- `email` is lowercase. `seen` counts mail from others that came to it; `blocked` sends
-- mail to it to Blocked. Signatures of its own only when `own_signatures` is set; the
-- address uses its account's otherwise.
CREATE TABLE account_addresses (
    account_id     TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    email          TEXT NOT NULL,
    name           TEXT,
    state          TEXT NOT NULL CHECK (state IN ('added', 'suggested', 'dismissed')),
    source         TEXT NOT NULL CHECK (source IN ('you', 'mail', 'gmail')),
    seen           INTEGER NOT NULL DEFAULT 0,
    blocked        INTEGER NOT NULL DEFAULT 0,
    position       INTEGER NOT NULL,
    own_signatures INTEGER NOT NULL DEFAULT 0,
    new_emails     TEXT REFERENCES signatures(id) ON DELETE SET NULL,
    replies        TEXT REFERENCES signatures(id) ON DELETE SET NULL,
    PRIMARY KEY (account_id, email)
);

-- NULL: from the account's own address.
ALTER TABLE drafts ADD COLUMN from_address TEXT;
"#;

/// Summaries in the kind you pick, kept per model and newest message (see `summary_cache.rs`);
/// "always summarise mail from…" rules (see `summary_rules.rs`); and why each automatic
/// one-liner was written. Turning those off is the `summaries:automatic` setting.
const V16: &str = r#"
-- Why the automatic one-liner was written: 'important' (in Important) or 'replied'
-- (you've written in it).
ALTER TABLE summaries ADD COLUMN why TEXT NOT NULL DEFAULT 'important' CHECK (why IN ('important', 'replied'));
UPDATE summaries SET why = 'replied'
    WHERE EXISTS (SELECT 1 FROM messages WHERE thread_id = summaries.thread_id AND from_me = 1);

-- A summary of each kind (and custom instruction) per writing model. `through` is the newest
-- message it read; `content` is the JSON the UI renders; `why` is 'asked' or 'rule'.
CREATE TABLE thread_summaries (
    thread_id   INTEGER NOT NULL REFERENCES threads(id) ON DELETE CASCADE,
    kind        TEXT NOT NULL CHECK (kind IN ('short', 'detailed', 'actions', 'facts', 'custom')),
    instruction TEXT NOT NULL DEFAULT '',
    model       TEXT NOT NULL,
    through     INTEGER NOT NULL,
    content     TEXT NOT NULL,
    why         TEXT NOT NULL CHECK (why IN ('asked', 'rule')),
    made_at     INTEGER NOT NULL,
    PRIMARY KEY (thread_id, kind, instruction, model)
);

-- "Always summarise mail from `sender` as `kind`", in every account. `sender` is a lowercase
-- address, or a domain (no @) for it and its subdomains. Mail that arrives after the rule
-- (a message id above `after_message`) is summarised in the background.
CREATE TABLE summary_rules (
    id            INTEGER PRIMARY KEY,
    sender        TEXT NOT NULL UNIQUE,
    kind          TEXT NOT NULL CHECK (kind IN ('short', 'detailed', 'actions', 'facts', 'custom')),
    instruction   TEXT NOT NULL DEFAULT '',
    after_message INTEGER NOT NULL,
    created_at    INTEGER NOT NULL
);
"#;

/// One summary per conversation: the kind you chose last replaces the one before (only the
/// newest is kept on upgrading). A summary you discard isn't written again in the background
/// until a newer message than `through` comes.
const V17: &str = r#"
CREATE TABLE thread_summary (
    thread_id   INTEGER PRIMARY KEY REFERENCES threads(id) ON DELETE CASCADE,
    kind        TEXT NOT NULL CHECK (kind IN ('short', 'detailed', 'actions', 'facts', 'custom')),
    instruction TEXT NOT NULL DEFAULT '',
    model       TEXT NOT NULL,
    through     INTEGER NOT NULL,
    content     TEXT NOT NULL,
    why         TEXT NOT NULL CHECK (why IN ('asked', 'rule')),
    made_at     INTEGER NOT NULL
);
INSERT INTO thread_summary
    SELECT thread_id, kind, instruction, model, through, content, why, made_at FROM thread_summaries s
    WHERE NOT EXISTS (SELECT 1 FROM thread_summaries o WHERE o.thread_id = s.thread_id
        AND (o.made_at > s.made_at OR (o.made_at = s.made_at AND o.rowid > s.rowid)));
DROP TABLE thread_summaries;
ALTER TABLE thread_summary RENAME TO thread_summaries;

CREATE TABLE discarded_summaries (
    thread_id INTEGER PRIMARY KEY REFERENCES threads(id) ON DELETE CASCADE,
    through   INTEGER NOT NULL
);
"#;

/// The text inside attachments, read on this Mac (see `attachment_text.rs`), so search and
/// Ask Moss can use it. The files themselves stay on the server; only their text is kept,
/// compressed like bodies, and indexed by its words and its meaning.
const V18: &str = r#"
-- One row per file read, even one with no words in it, so each file is read once.
CREATE TABLE attachment_text (
    id         INTEGER PRIMARY KEY,
    message_id INTEGER NOT NULL,
    position   INTEGER NOT NULL,
    method     TEXT NOT NULL,
    text       BLOB NOT NULL,
    chars      INTEGER NOT NULL,
    read_at    INTEGER NOT NULL,
    UNIQUE (message_id, position),
    FOREIGN KEY (message_id, position) REFERENCES attachments (message_id, position) ON DELETE CASCADE
);

-- Which words are in which file's text, by `attachment_text.id`. Like `message_search`,
-- it keeps no readable text.
CREATE VIRTUAL TABLE attachment_search USING fts5 (
    text,
    content = '', contentless_delete = 1,
    tokenize = 'unicode61 remove_diacritics 2'
);
CREATE TRIGGER attachment_text_search_delete AFTER DELETE ON attachment_text BEGIN
    DELETE FROM attachment_search WHERE rowid = old.id;
END;

-- What each file's text means, as `embeddings` holds for messages.
CREATE TABLE attachment_embeddings (
    text_id INTEGER PRIMARY KEY REFERENCES attachment_text(id) ON DELETE CASCADE,
    model   TEXT NOT NULL,
    vector  BLOB NOT NULL
);
"#;

/// Emails set to send later (see `outbox.rs`), kept on this Mac until they go: the draft to
/// open again, and the email the engine sends. A failed try is tried again from `next_try`.
const V19: &str = r#"
CREATE TABLE scheduled (
    draft_id   TEXT PRIMARY KEY,
    account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    draft      TEXT NOT NULL,
    outgoing   TEXT NOT NULL,
    send_at    INTEGER NOT NULL,
    next_try   INTEGER NOT NULL,
    tries      INTEGER NOT NULL DEFAULT 0,
    error      TEXT
);
CREATE INDEX scheduled_by_next_try ON scheduled (next_try);
"#;

/// Files you attach to a draft, kept here until it is sent, so they survive closing the
/// composer and restarting. The draft lists its files, these and forwarded ones, in
/// `files_json`. A file stays when its draft is deleted on Send, until the email has gone.
const V20: &str = r#"
ALTER TABLE drafts ADD COLUMN files_json TEXT NOT NULL DEFAULT '[]';

CREATE TABLE draft_files (
    id           INTEGER PRIMARY KEY,
    draft_id     TEXT NOT NULL,
    name         TEXT NOT NULL,
    content_type TEXT NOT NULL,
    data         BLOB NOT NULL,
    added_at     INTEGER NOT NULL
);
CREATE INDEX draft_files_by_draft ON draft_files (draft_id);
"#;

/// V21. Drafts live in each account's Drafts folder too (see `drafts.rs`). `server_message_id` is
/// the Message-ID of the copy there and `server_provider_id` its id in that folder, once
/// known; `pushed_at` is the `updated_at` that copy holds. A draft whose `pushed_at` isn't
/// its `updated_at` goes up on the next sync, so drafts from before this go up too.
const V21: &str = r#"
ALTER TABLE drafts ADD COLUMN server_message_id TEXT;
ALTER TABLE drafts ADD COLUMN server_provider_id TEXT;
ALTER TABLE drafts ADD COLUMN pushed_at INTEGER;
"#;

/// Every decision Moss makes about a message, not only the model's: who made it (the model,
/// a rule, or you), and its detail, such as a split or a due day (see `decisions.rs`). Your
/// answers are never asked again or overwritten. Earlier "deadline" and "category" answers
/// held no detail, so they go and are asked again.
const V22: &str = r#"
CREATE TABLE decisions (
    message_id INTEGER NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
    question   TEXT NOT NULL,
    -- How likely the decision model said yes, or that its pick is right. NULL until it is asked.
    yes        REAL,
    -- Who gave the answer Moss goes by.
    source     TEXT NOT NULL CHECK (source IN ('model', 'rule', 'you')),
    -- Whether Moss acts on it: moved the mail, shows the due day, brings it back.
    acted      INTEGER NOT NULL,
    -- The answer: a section or split name, a due day (YYYY-MM-DD), a time to come back (ms).
    value      TEXT,
    -- What a rule went by, in the email's own words, e.g. "by Friday".
    evidence   TEXT,
    decided_at INTEGER NOT NULL,
    PRIMARY KEY (message_id, question)
);
INSERT INTO decisions (message_id, question, yes, source, acted, value, decided_at)
    SELECT d.message_id, d.question, d.yes, 'model', d.acted, CASE WHEN d.acted THEN m.section END, d.decided_at
    FROM ai_decisions d JOIN messages m ON m.id = d.message_id WHERE d.question = 'needsReply';
DROP TABLE ai_decisions;
ALTER TABLE decisions RENAME TO ai_decisions;
"#;

/// The split tab you moved a thread to yourself, other than Important (a star on its latest
/// email says that, on the server too). NULL: wherever Moss's sorting puts it. See `moves.rs`.
const V23: &str = r#"
ALTER TABLE threads ADD COLUMN moved_to TEXT
    CHECK (moved_to IN ('other', 'calendar', 'newsletters', 'receipts'));
"#;

/// V24. A file you attach never gets the id of one that went: its thumbnail is kept by id,
/// and a new file showed the old one's. AUTOINCREMENT keeps counting past deleted rows.
const V24: &str = r#"
CREATE TABLE draft_files_v24 (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    draft_id     TEXT NOT NULL,
    name         TEXT NOT NULL,
    content_type TEXT NOT NULL,
    data         BLOB NOT NULL,
    added_at     INTEGER NOT NULL
);
INSERT INTO draft_files_v24 (id, draft_id, name, content_type, data, added_at)
    SELECT id, draft_id, name, content_type, data, added_at FROM draft_files;
DROP TABLE draft_files;
ALTER TABLE draft_files_v24 RENAME TO draft_files;
CREATE INDEX draft_files_by_draft ON draft_files (draft_id);
"#;

/// V25. Durable delivery evidence, independent of the UI and scheduler lifetimes.
const V25: &str = r#"
ALTER TABLE scheduled ADD COLUMN operation_id TEXT;
UPDATE scheduled SET operation_id = lower(hex(randomblob(16)));
CREATE UNIQUE INDEX scheduled_operation ON scheduled(operation_id);
-- Keep only the latest manual snooze/wake per thread. AUTOINCREMENT retains ordering
-- across replacement and deletion, without relying on wall-clock timestamps.
CREATE TABLE snooze_edits (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    thread_id INTEGER NOT NULL UNIQUE REFERENCES threads(id) ON DELETE CASCADE
);
CREATE TABLE deliveries (
    id TEXT PRIMARY KEY,
    draft_id TEXT NOT NULL,
    account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    draft TEXT NOT NULL,
    outgoing TEXT NOT NULL,
    message_id TEXT NOT NULL,
    state TEXT NOT NULL CHECK (state IN ('held', 'claimed', 'accepted', 'uncertain', 'complete', 'cancelled')),
    raw BLOB,
    source TEXT,
    thread_id TEXT,
    error TEXT,
    created_at INTEGER NOT NULL,
    reminder_applied INTEGER NOT NULL DEFAULT 0 CHECK (reminder_applied IN (0, 1)),
    snooze_revision INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX deliveries_pending ON deliveries(state, created_at);
CREATE UNIQUE INDEX deliveries_own_draft ON deliveries(draft_id) WHERE state != 'cancelled';
"#;

/// V26. Undo may arrive before preparation. Retain its operation identity across restarts.
const V26: &str = r#"
CREATE TABLE delivery_cancellations (
    id TEXT PRIMARY KEY NOT NULL
);
"#;

/// V27. A schedule's terminal identity survives removal of its active row.
const V27: &str = r#"
CREATE TABLE schedule_outcomes (
    id TEXT PRIMARY KEY NOT NULL,
    draft_id TEXT NOT NULL,
    outcome TEXT NOT NULL CHECK (outcome IN ('cancelled', 'completed')),
    at INTEGER NOT NULL
);
ALTER TABLE scheduled ADD COLUMN request_digest TEXT;
"#;

/// V28. User-intent revisions prevent stale reschedules from overwriting newer times.
const V28: &str = r#"
ALTER TABLE scheduled ADD COLUMN revision INTEGER NOT NULL DEFAULT 1
    CHECK (revision BETWEEN 1 AND 9007199254740991);
"#;

/// V29. Caller request identities are retained independently of account deletion.
/// Staging and handoff are not exposed until their lifecycle protocol is integrated.
const V29: &str = r#"
CREATE TABLE agent_requests (
    client TEXT NOT NULL CHECK (length(client) BETWEEN 1 AND 256),
    request_key TEXT NOT NULL CHECK (length(request_key) = 64 AND request_key NOT GLOB '*[^0-9a-f]*'),
    kind INTEGER NOT NULL CHECK (kind IN (0, 1)),
    fingerprint_version INTEGER NOT NULL CHECK (fingerprint_version BETWEEN 1 AND 4294967295),
    fingerprint TEXT NOT NULL CHECK (length(fingerprint) = 64 AND fingerprint NOT GLOB '*[^0-9a-f]*'),
    draft_id TEXT NOT NULL UNIQUE CHECK (length(draft_id) = 32),
    operation_id TEXT NOT NULL UNIQUE CHECK (length(operation_id) = 32),
    created_at INTEGER NOT NULL,
    stage TEXT NOT NULL DEFAULT 'reserved' CHECK (stage IN ('reserved', 'drafted', 'handedOff', 'cancelled')),
    account_id TEXT,
    staged_at INTEGER,
    cancelled_at INTEGER,
    cancel_reason TEXT CHECK (cancel_reason IN ('user', 'accountRemoved', 'failed')),
    snapshot TEXT,
    CHECK ((stage = 'cancelled' AND cancelled_at IS NOT NULL AND cancel_reason IS NOT NULL)
        OR (stage != 'cancelled' AND cancelled_at IS NULL AND cancel_reason IS NULL)),
    CHECK ((stage = 'reserved' AND account_id IS NULL AND snapshot IS NULL AND staged_at IS NULL)
        OR (stage = 'drafted' AND account_id IS NOT NULL AND snapshot IS NOT NULL AND json_valid(snapshot) AND staged_at IS NOT NULL)
        OR (stage = 'handedOff' AND snapshot IS NULL)
        OR stage = 'cancelled'),
    PRIMARY KEY (client, request_key)
);
CREATE INDEX agent_requests_attention ON agent_requests(created_at, operation_id) WHERE stage != 'handedOff';
"#;

/// V30. Content revisions are opaque equality tokens, independent of wall-clock time.
const V30: &str = r#"
ALTER TABLE drafts ADD COLUMN revision TEXT NOT NULL DEFAULT ''
    CHECK (revision = '' OR (length(revision) = 32 AND revision NOT GLOB '*[^0-9a-f]*'));
ALTER TABLE drafts ADD COLUMN pushed_revision TEXT
    CHECK (pushed_revision IS NULL OR (length(pushed_revision) = 32 AND pushed_revision NOT GLOB '*[^0-9a-f]*'));
UPDATE drafts SET revision = lower(hex(randomblob(16)));
UPDATE drafts SET pushed_revision = revision
    WHERE server_message_id IS NOT NULL AND pushed_at = updated_at;
CREATE UNIQUE INDEX drafts_revision ON drafts(revision) WHERE revision != '';
CREATE TRIGGER drafts_revision_insert AFTER INSERT ON drafts BEGIN
    UPDATE drafts SET revision = lower(hex(randomblob(16))) WHERE id = NEW.id;
END;
CREATE TRIGGER drafts_revision_update AFTER UPDATE OF mode, thread_id, account_id, to_json, cc_json, bcc_json, subject, body, body_html, remind_if_no_reply, signature_id, signature_chosen, from_address, files_json ON drafts
WHEN OLD.mode IS NOT NEW.mode OR OLD.thread_id IS NOT NEW.thread_id OR OLD.account_id IS NOT NEW.account_id OR OLD.to_json IS NOT NEW.to_json OR OLD.cc_json IS NOT NEW.cc_json OR OLD.bcc_json IS NOT NEW.bcc_json OR OLD.subject IS NOT NEW.subject OR OLD.body IS NOT NEW.body OR OLD.body_html IS NOT NEW.body_html OR OLD.remind_if_no_reply IS NOT NEW.remind_if_no_reply OR OLD.signature_id IS NOT NEW.signature_id OR OLD.signature_chosen IS NOT NEW.signature_chosen OR OLD.from_address IS NOT NEW.from_address OR OLD.files_json IS NOT NEW.files_json
BEGIN
    UPDATE drafts SET revision = lower(hex(randomblob(16))) WHERE id = NEW.id;
END;
"#;

/// V31. Ambiguous remote content is recovery data, never a sendable draft.
/// No account foreign key: deleting an account must not erase unresolved evidence.
const V31: &str = r#"
CREATE TABLE account_incarnation_history (
    token TEXT PRIMARY KEY CHECK(length(token)=32 AND token NOT GLOB '*[^0-9a-f]*'),
    account_id TEXT NOT NULL
);
CREATE TABLE account_incarnations (
    account_id TEXT PRIMARY KEY REFERENCES accounts(id) ON DELETE CASCADE,
    token TEXT NOT NULL UNIQUE CHECK(length(token)=32 AND token NOT GLOB '*[^0-9a-f]*')
);
INSERT INTO account_incarnations SELECT id, lower(hex(randomblob(16))) FROM accounts;
INSERT INTO account_incarnation_history SELECT token,account_id FROM account_incarnations;
CREATE TRIGGER account_incarnation_insert AFTER INSERT ON accounts BEGIN
    INSERT INTO account_incarnations(account_id,token) VALUES(NEW.id,lower(hex(randomblob(16))));
    INSERT INTO account_incarnation_history SELECT token,account_id FROM account_incarnations WHERE account_id=NEW.id;
END;
CREATE TABLE remote_draft_quarantine (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    account_id TEXT NOT NULL CHECK(length(CAST(account_id AS BLOB)) BETWEEN 1 AND 1024),
    account_incarnation TEXT NOT NULL CHECK(length(account_incarnation)=32 AND account_incarnation NOT GLOB '*[^0-9a-f]*'),
    mailbox TEXT NOT NULL CHECK(length(CAST(mailbox AS BLOB)) BETWEEN 1 AND 4096),
    provider_id TEXT NOT NULL CHECK(length(CAST(provider_id AS BLOB)) BETWEEN 1 AND 1024),
    raw_sha256 TEXT NOT NULL CHECK(typeof(raw_sha256)='text' AND length(raw_sha256)=64 AND raw_sha256 NOT GLOB '*[^0-9a-f]*'),
    raw BLOB NOT NULL CHECK(typeof(raw)='blob' AND length(raw) <= 104857600),
    reason TEXT NOT NULL CHECK(reason IN ('sent','uncertain','scheduled','staged','otherAccount','conflict','unverified')),
    related_id TEXT CHECK(related_id IS NULL OR length(CAST(related_id AS BLOB)) BETWEEN 1 AND 1024),
    state TEXT NOT NULL DEFAULT 'open' CHECK(state IN ('open','copiedToDraft','dismissed','serverDeleted')),
    created_at INTEGER NOT NULL,
    last_seen_at INTEGER NOT NULL CHECK(last_seen_at >= created_at),
    resolved_at INTEGER CHECK(resolved_at IS NULL OR resolved_at >= created_at),
    CHECK((state='open' AND resolved_at IS NULL) OR (state<>'open' AND resolved_at IS NOT NULL)),
    UNIQUE(account_id, account_incarnation, mailbox, provider_id, raw_sha256)
);
CREATE INDEX remote_draft_attention ON remote_draft_quarantine(id)
    WHERE state NOT IN ('dismissed','copiedToDraft','serverDeleted');
"#;

/// Queue receipts identify one immutable action, independently of reusable row numbers.
const V32: &str = r#"
ALTER TABLE outbox RENAME TO outbox_before_v32;
CREATE TABLE outbox (
    id INTEGER PRIMARY KEY,
    account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    change TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    token BLOB NOT NULL UNIQUE DEFAULT(randomblob(16)) CHECK(typeof(token)='blob' AND length(token)=16)
);
INSERT INTO outbox(id,account_id,change,created_at)
    SELECT id,account_id,change,created_at FROM outbox_before_v32 ORDER BY id;
DROP TABLE outbox_before_v32;
CREATE INDEX outbox_by_account ON outbox(account_id,id);
CREATE TRIGGER outbox_payload_immutable BEFORE UPDATE OF id,account_id,change,created_at,token ON outbox BEGIN
    SELECT RAISE(ABORT, 'A queued action cannot be rewritten.');
END;
"#;

/// Queued actions retain failed intent and a crash-surviving attempt claim.
const V33: &str = r#"
ALTER TABLE outbox ADD COLUMN state TEXT NOT NULL DEFAULT 'queued' CHECK(state IN ('queued','claimed','blocked'));
ALTER TABLE outbox ADD COLUMN state_revision INTEGER NOT NULL DEFAULT 0
    CHECK(typeof(state_revision)='integer' AND state_revision BETWEEN 0 AND 9007199254740991);
ALTER TABLE outbox ADD COLUMN failure TEXT
    CHECK((state='blocked' AND failure IS NOT NULL AND failure IN ('refused','unreadable','uncertain','tooLarge'))
       OR (state IN ('queued','claimed') AND failure IS NULL));
"#;

/// One logical action retains each part's explicit outcome until all work is resolved.
const V34: &str = r#"
CREATE TABLE outbox_batches (
    id BLOB PRIMARY KEY CHECK(typeof(id)='blob' AND length(id)=16),
    account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    kind TEXT NOT NULL CHECK(kind IN ('legacy','done','undone','read','delete','restore','move','forwarded','draftCleanup')),
    parts INTEGER NOT NULL CHECK(typeof(parts)='integer' AND parts BETWEEN 1 AND 64),
    created_at INTEGER NOT NULL
);
CREATE TABLE outbox_batch_parts (
    token BLOB PRIMARY KEY CHECK(typeof(token)='blob' AND length(token)=16),
    batch BLOB NOT NULL REFERENCES outbox_batches(id) ON DELETE CASCADE,
    ordinal INTEGER NOT NULL CHECK(typeof(ordinal)='integer' AND ordinal BETWEEN 0 AND 63),
    outcome TEXT NOT NULL DEFAULT 'pending' CHECK(outcome IN ('pending','completed','discarded','dropped')),
    UNIQUE(batch,ordinal)
);
CREATE INDEX outbox_batches_account ON outbox_batches(account_id);
INSERT INTO outbox_batches(id,account_id,kind,parts,created_at)
    SELECT token,account_id,'legacy',1,created_at FROM outbox;
INSERT INTO outbox_batch_parts(token,batch,ordinal) SELECT token,token,0 FROM outbox;
CREATE TRIGGER outbox_batch_immutable BEFORE UPDATE ON outbox_batches BEGIN
    SELECT RAISE(ABORT, 'A server action batch cannot be rewritten.');
END;
CREATE TRIGGER outbox_part_immutable BEFORE UPDATE OF token,batch,ordinal ON outbox_batch_parts BEGIN
    SELECT RAISE(ABORT, 'A server action part cannot be rewritten.');
END;
CREATE TRIGGER outbox_part_position BEFORE INSERT ON outbox_batch_parts
WHEN NOT EXISTS(SELECT 1 FROM outbox_batches WHERE id=NEW.batch AND NEW.ordinal<parts) BEGIN
    SELECT RAISE(ABORT, 'The server action part is outside its batch.');
END;
CREATE TRIGGER outbox_part_outcome BEFORE UPDATE OF outcome ON outbox_batch_parts
WHEN OLD.outcome<>'pending' OR NEW.outcome NOT IN ('completed','discarded','dropped') BEGIN
    SELECT RAISE(ABORT, 'The server action part is already resolved.');
END;
CREATE TRIGGER outbox_batch_insert AFTER INSERT ON outbox BEGIN
    INSERT INTO outbox_batches(id,account_id,kind,parts,created_at)
        SELECT NEW.token,NEW.account_id,'legacy',1,NEW.created_at
        WHERE NOT EXISTS(SELECT 1 FROM outbox_batch_parts WHERE token=NEW.token);
    INSERT INTO outbox_batch_parts(token,batch,ordinal)
        SELECT NEW.token,NEW.token,0 WHERE NOT EXISTS(SELECT 1 FROM outbox_batch_parts WHERE token=NEW.token);
    SELECT CASE WHEN NOT EXISTS(
        SELECT 1 FROM outbox_batch_parts p JOIN outbox_batches b ON b.id=p.batch
        WHERE p.token=NEW.token AND p.outcome='pending' AND b.account_id=NEW.account_id AND p.ordinal<b.parts
    ) THEN RAISE(ABORT, 'The server action batch is inconsistent.') END;
END;
CREATE TRIGGER outbox_batch_delete AFTER DELETE ON outbox BEGIN
    UPDATE outbox_batch_parts SET outcome='dropped' WHERE token=OLD.token AND outcome='pending';
    DELETE FROM outbox_batches WHERE id=(SELECT batch FROM outbox_batch_parts WHERE token=OLD.token)
        AND NOT EXISTS(SELECT 1 FROM outbox_batch_parts p WHERE p.batch=outbox_batches.id AND p.outcome='pending')
        AND NOT EXISTS(SELECT 1 FROM outbox_batch_parts p JOIN outbox o ON o.token=p.token WHERE p.batch=outbox_batches.id);
END;
"#;

/// References only. These survive account deletion until Keychain cleanup is verified.
const V35: &str = r#"
CREATE TABLE credential_cleanup (
    service TEXT NOT NULL CHECK(length(service) BETWEEN 1 AND 256),
    account TEXT NOT NULL CHECK(length(account) BETWEEN 1 AND 4096),
    PRIMARY KEY(service, account)
);
CREATE TRIGGER credential_cleanup_immutable BEFORE UPDATE ON credential_cleanup BEGIN
    SELECT RAISE(ABORT, 'A credential cleanup reference cannot be rewritten.');
END;
"#;

/// Keyset scans for background meaning-index backfill, including equal timestamps.
const V36: &str = r#"
CREATE INDEX messages_by_sent_at ON messages(sent_at DESC, id DESC);
"#;

/// V37. External insertion cannot reuse an identity after deletion or handoff.
/// Minimal identity records intentionally survive account removal and draft deletion.
const V37: &str = r#"
CREATE TABLE draft_identities (id TEXT PRIMARY KEY NOT NULL);
INSERT OR IGNORE INTO draft_identities SELECT id FROM drafts;
INSERT OR IGNORE INTO draft_identities SELECT draft_id FROM deliveries;
INSERT OR IGNORE INTO draft_identities SELECT draft_id FROM scheduled;
INSERT OR IGNORE INTO draft_identities SELECT draft_id FROM schedule_outcomes WHERE outcome='completed';
CREATE TRIGGER draft_identity_insert AFTER INSERT ON drafts BEGIN
    INSERT OR IGNORE INTO draft_identities(id) VALUES (NEW.id);
END;
"#;

/// Retain an editor precondition as receipt metadata, separate from frozen content.
const V38: &str = r#"
ALTER TABLE deliveries ADD COLUMN source_revision TEXT;
"#;

/// V39. Distinguish cancellation before ownership transfer from a retired restoration.
const V39: &str = r#"
ALTER TABLE schedule_outcomes ADD COLUMN never_prepared INTEGER NOT NULL DEFAULT 0 CHECK (never_prepared IN (0, 1));
CREATE INDEX schedule_recovery_draft ON schedule_outcomes(draft_id) WHERE never_prepared=1;
"#;


/// Local conflict copies are deliberately separate from editable and outgoing drafts.
/// No account foreign key: removing an account must not silently destroy unsaved text.
const V40: &str = r#"
CREATE TABLE local_draft_recoveries (
    id TEXT PRIMARY KEY NOT NULL,
    original_id TEXT NOT NULL,
    base_revision TEXT,
    version TEXT NOT NULL,
    snapshot TEXT,
    snapshot_bytes INTEGER NOT NULL CHECK(snapshot_bytes >= 0),
    state TEXT NOT NULL DEFAULT 'open' CHECK(state IN ('open','adopted','discarded')),
    adopted_id TEXT,
    adoption_request TEXT,
    updated_at INTEGER NOT NULL,
    CHECK(snapshot_bytes = coalesce(length(CAST(snapshot AS BLOB)), 0)),
    CHECK((state='open' AND snapshot IS NOT NULL AND adopted_id IS NULL AND adoption_request IS NULL)
       OR (state='adopted' AND snapshot IS NULL AND adopted_id IS NOT NULL AND adoption_request IS NOT NULL)
       OR (state='discarded' AND snapshot IS NULL AND adopted_id IS NULL AND adoption_request IS NULL))
);
CREATE INDEX local_recovery_open ON local_draft_recoveries(updated_at DESC, id) WHERE state='open';
CREATE TABLE local_draft_recovery_files (
    recovery_id TEXT NOT NULL REFERENCES local_draft_recoveries(id) ON DELETE CASCADE,
    source_id TEXT NOT NULL,
    name TEXT NOT NULL,
    content_type TEXT NOT NULL,
    bytes INTEGER NOT NULL CHECK(bytes >= 0 AND bytes=length(data)),
    sha256 TEXT NOT NULL,
    data BLOB NOT NULL,
    PRIMARY KEY(recovery_id, source_id)
);
CREATE UNIQUE INDEX local_recovery_adopted ON local_draft_recoveries(adopted_id) WHERE adopted_id IS NOT NULL;
CREATE TRIGGER local_recovery_files_insert BEFORE INSERT ON local_draft_recovery_files
WHEN NOT EXISTS(SELECT 1 FROM local_draft_recoveries WHERE id=NEW.recovery_id AND state='open')
BEGIN SELECT RAISE(ABORT, 'Recovery files require an open owner'); END;
CREATE TRIGGER local_recovery_files_update BEFORE UPDATE ON local_draft_recovery_files
WHEN NOT EXISTS(SELECT 1 FROM local_draft_recoveries WHERE id=NEW.recovery_id AND state='open')
BEGIN SELECT RAISE(ABORT, 'Recovery files require an open owner'); END;
CREATE TRIGGER local_recovery_resolve BEFORE UPDATE OF state ON local_draft_recoveries
WHEN NEW.state!='open' AND EXISTS(SELECT 1 FROM local_draft_recovery_files WHERE recovery_id=OLD.id)
BEGIN SELECT RAISE(ABORT, 'Recovery files must be released before resolution'); END;
"#;

/// Local receipt repair is bounded and backs off durably across process restarts.
const V41: &str = r#"
ALTER TABLE deliveries ADD COLUMN interrupted_from TEXT CHECK(interrupted_from IN ('held','claimed'));
ALTER TABLE deliveries ADD COLUMN repair_next INTEGER NOT NULL DEFAULT 0;
ALTER TABLE deliveries ADD COLUMN repair_attempt INTEGER NOT NULL DEFAULT 0 CHECK(repair_attempt BETWEEN 0 AND 4);
CREATE INDEX delivery_repair_due ON deliveries(repair_next, id) WHERE state='accepted';
"#;

/// Terminal send identities survive account deletion. Unresolved evidence is still
/// protected by the account-removal guard; new preparation checks account ownership.
const V42: &str = r#"
CREATE TABLE deliveries_v42 (
    id TEXT PRIMARY KEY,
    draft_id TEXT NOT NULL,
    account_id TEXT NOT NULL,
    draft TEXT NOT NULL,
    outgoing TEXT NOT NULL,
    message_id TEXT NOT NULL,
    state TEXT NOT NULL CHECK (state IN ('held','claimed','accepted','uncertain','complete','cancelled')),
    raw BLOB,
    source TEXT,
    thread_id TEXT,
    error TEXT,
    created_at INTEGER NOT NULL,
    reminder_applied INTEGER NOT NULL DEFAULT 0 CHECK (reminder_applied IN (0,1)),
    snooze_revision INTEGER NOT NULL DEFAULT 0,
    source_revision TEXT,
    interrupted_from TEXT CHECK (interrupted_from IN ('held','claimed')),
    repair_next INTEGER NOT NULL DEFAULT 0,
    repair_attempt INTEGER NOT NULL DEFAULT 0 CHECK (repair_attempt BETWEEN 0 AND 4),
    account_retired INTEGER NOT NULL DEFAULT 0 CHECK (account_retired IN (0,1))
);
INSERT INTO deliveries_v42 (id,draft_id,account_id,draft,outgoing,message_id,state,raw,source,thread_id,error,created_at,reminder_applied,snooze_revision,source_revision,interrupted_from,repair_next,repair_attempt)
    SELECT id,draft_id,account_id,draft,outgoing,message_id,state,raw,source,thread_id,error,created_at,reminder_applied,snooze_revision,source_revision,interrupted_from,repair_next,repair_attempt FROM deliveries;
DROP TABLE deliveries;
ALTER TABLE deliveries_v42 RENAME TO deliveries;
CREATE INDEX deliveries_pending ON deliveries(state,created_at);
CREATE UNIQUE INDEX deliveries_own_draft ON deliveries(draft_id) WHERE state!='cancelled';
CREATE INDEX delivery_repair_due ON deliveries(repair_next,id) WHERE state='accepted';
"#;

/// Explicit review resolves recovery ownership without changing delivery certainty.
const V43: &str = r#"
ALTER TABLE deliveries ADD COLUMN file_ceiling INTEGER NOT NULL DEFAULT 0 CHECK(file_ceiling>=0);
UPDATE deliveries SET file_ceiling=coalesce((
    SELECT max(f.id) FROM draft_files f
    JOIN json_each(CASE WHEN length(CAST(deliveries.draft AS BLOB))<=16777216 AND json_valid(deliveries.draft)
        THEN deliveries.draft ELSE '{}' END,'$.attachments') a ON CAST(f.id AS TEXT)=json_extract(CASE WHEN a.type='object' THEN a.value ELSE '{}' END,'$.id')
    WHERE f.draft_id=deliveries.draft_id),0);
ALTER TABLE deliveries ADD COLUMN cancel_intent INTEGER NOT NULL DEFAULT 0 CHECK(cancel_intent IN (0,1));
UPDATE deliveries SET cancel_intent=1 WHERE EXISTS(SELECT 1 FROM delivery_cancellations WHERE delivery_cancellations.id=deliveries.id);
ALTER TABLE deliveries ADD COLUMN resolution TEXT CHECK(resolution IN ('reviewed','copied'));
ALTER TABLE deliveries ADD COLUMN recovery_id TEXT;
ALTER TABLE deliveries ADD COLUMN resolved_at INTEGER CHECK(
    (resolution IS NULL AND resolved_at IS NULL AND recovery_id IS NULL) OR
    (resolution='reviewed' AND resolved_at IS NOT NULL AND recovery_id IS NULL) OR
    (resolution='copied' AND resolved_at IS NOT NULL AND recovery_id IS NOT NULL));
CREATE INDEX delivery_attention_pending ON deliveries(created_at,id)
    WHERE resolution IS NULL AND (state IN ('accepted','uncertain') OR (state='held' AND cancel_intent=1));
"#;

/// Terminal payload retirement retains operation identity and versioned replay proofs.
const V44: &str = r#"
ALTER TABLE deliveries ADD COLUMN storage_hash TEXT;
ALTER TABLE deliveries ADD COLUMN logical_hash TEXT;
ALTER TABLE deliveries ADD COLUMN compacted_at INTEGER CHECK(compacted_at IS NULL OR state IN ('complete','cancelled') OR resolution IS NOT NULL);
CREATE INDEX delivery_compaction_due ON deliveries(account_retired DESC,created_at,id)
    WHERE compacted_at IS NULL AND (state IN ('complete','cancelled') OR resolution IS NOT NULL);
"#;

/// Bound empty caller reservations without scanning retained terminal identities.
const V45: &str = r#"
CREATE INDEX agent_requests_reserved_owner ON agent_requests(client) WHERE stage='reserved';
"#;

/// V46. Lookups that a big mailbox can afford; nothing new to see. `sent_to` is who you wrote
/// to (To, Cc or Bcc): one row per address on each email you sent, lowercase, with its domain
/// (after the last @; NULL without one). Triggers keep it in step with `messages`, so asking
/// "did you write to them?" no longer reads every email you sent.
const V46: &str = r#"
CREATE TABLE sent_to (
    message_id INTEGER NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
    account_id TEXT NOT NULL,
    email      TEXT NOT NULL,
    domain     TEXT,
    PRIMARY KEY (message_id, email)
);
CREATE INDEX sent_to_by_email ON sent_to (account_id, email);
CREATE INDEX sent_to_by_domain ON sent_to (account_id, domain) WHERE domain IS NOT NULL;
-- Recipients that aren't an object with an address, or a list that isn't JSON, are left out.
CREATE VIEW sent_to_source AS
    SELECT message_id, account_id, email,
           CASE WHEN instr(email, '@') THEN substr(email, length(rtrim(email, replace(email, '@', ''))) + 1) END AS domain
    FROM (SELECT m.id AS message_id, m.account_id, lower(json_extract(j.value, '$.email')) AS email
              FROM messages m, json_each(CASE WHEN json_valid(m.to_json) THEN m.to_json ELSE '[]' END) j
              WHERE m.from_me = 1 AND j.type = 'object'
          UNION ALL SELECT m.id, m.account_id, lower(json_extract(j.value, '$.email'))
              FROM messages m, json_each(CASE WHEN json_valid(m.cc_json) THEN m.cc_json ELSE '[]' END) j
              WHERE m.from_me = 1 AND j.type = 'object'
          UNION ALL SELECT m.id, m.account_id, lower(json_extract(j.value, '$.email'))
              FROM messages m, json_each(CASE WHEN json_valid(m.bcc_json) THEN m.bcc_json ELSE '[]' END) j
              WHERE m.from_me = 1 AND j.type = 'object')
    WHERE email IS NOT NULL;
INSERT OR IGNORE INTO sent_to (message_id, account_id, email, domain) SELECT * FROM sent_to_source;
CREATE TRIGGER sent_to_insert AFTER INSERT ON messages WHEN NEW.from_me BEGIN
    INSERT OR IGNORE INTO sent_to (message_id, account_id, email, domain)
        SELECT * FROM sent_to_source WHERE message_id = NEW.id;
END;
CREATE TRIGGER sent_to_update AFTER UPDATE OF account_id, from_me, to_json, cc_json, bcc_json ON messages BEGIN
    DELETE FROM sent_to WHERE message_id = OLD.id;
    INSERT OR IGNORE INTO sent_to (message_id, account_id, email, domain)
        SELECT * FROM sent_to_source WHERE message_id = NEW.id;
END;
-- Mail from someone, by address as sender rules and "known sender" compare it.
CREATE INDEX messages_by_sender ON messages (account_id, lower(from_email));
-- The newest mail first, for the decision model's queue.
CREATE INDEX messages_by_received_at ON messages (received_at);
-- The newest threads first across accounts, for Blocked and the summary queue.
CREATE INDEX threads_by_last_message ON threads (last_message_at);
"#;

/// V47. Whether the account's own mail server confirmed that a message comes from the domain
/// in its From line (see `authres.rs`): only then do images load because of who it's from.
/// Older mail's headers weren't kept, so it counts as unconfirmed (its images ask first), apart
/// from your own copies: what's in a Sent folder, and what Moss sent and keeps itself.
const V47: &str = r#"
ALTER TABLE messages ADD COLUMN sender_verified INTEGER NOT NULL DEFAULT 0 CHECK (sender_verified IN (0, 1));
UPDATE messages SET sender_verified = 1 WHERE id IN (
    SELECT l.message_id FROM message_locations l JOIN mailboxes b ON b.id = l.mailbox_id
    WHERE b.role = 'sent' OR l.provider_id LIKE 'local:%');
"#;

/// V48. Leftovers of subsystems that were removed before they were ever wired in: agent send
/// requests (V29, V45) and the quarantine of ambiguous server drafts with its account
/// incarnations (V31). Nothing writes them any more; their indexes go with their tables, and the
/// trigger on `accounts` goes first so adding an account never writes into a table that's gone.
const V48: &str = r#"
DROP TRIGGER account_incarnation_insert;
DROP TABLE account_incarnations;
DROP TABLE account_incarnation_history;
DROP TABLE remote_draft_quarantine;
DROP TABLE agent_requests;
"#;

/// V49. Emails Moss stopped before sending (a crash or force quit during Undo), back in Drafts,
/// until you have seen the note that says so. They go with their account.
const V49: &str = r#"
CREATE TABLE send_interruptions (
    id         TEXT PRIMARY KEY NOT NULL,
    account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    subject    TEXT NOT NULL,
    at         INTEGER NOT NULL
);
"#;

/// V50. Mail from a stranger in Important is now also asked whether it is a sales pitch. Mail
/// the decision model already left in Important was asked before that question existed, so
/// its split answer goes and the sorter asks again (only mail of the last 30 days is asked).
/// Splits it moved, list mail and your own moves stay as they are.
const V50: &str = r#"
DELETE FROM ai_decisions WHERE question = 'split' AND source = 'model' AND acted = 0
    AND message_id IN (SELECT id FROM messages WHERE split = 'important');
"#;

/// V51. Who you chose from the recipient suggestions for what you typed, so the next time
/// you type the same or similar the one you chose comes first.
const V51: &str = r#"
CREATE TABLE recipient_picks (
    typed TEXT NOT NULL,
    email TEXT NOT NULL,
    at    INTEGER NOT NULL,
    PRIMARY KEY (typed, email)
);
"#;

#[cfg(test)]
mod tests {
    use crate::test_support::*;
    use crate::Store;

    fn objects(store: &Store, names: &[&str]) -> Vec<String> {
        let mut statement = store.conn.prepare("SELECT name FROM sqlite_master ORDER BY name").unwrap();
        let all: Vec<String> = statement.query_map([], |r| r.get(0)).unwrap().collect::<rusqlite::Result<_>>().unwrap();
        all.into_iter().filter(|name| names.contains(&name.as_str())).collect()
    }

    const LEFTOVERS: &[&str] = &["agent_requests", "agent_requests_attention", "agent_requests_reserved_owner",
        "account_incarnation_insert", "account_incarnations", "account_incarnation_history",
        "remote_draft_quarantine", "remote_draft_attention"];

    #[test]
    fn v48_drops_the_tables_of_removed_subsystems_with_their_rows_and_keeps_the_mail() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("moss.db");
        {
            let mut old = historical_store(&path, 47);
            old.upsert_account(&account("work", "maya@example.test")).unwrap();
            ingest(&mut old, "1", &mail().build());
            old.conn.execute_batch(&format!("
                INSERT INTO agent_requests(client, request_key, kind, fingerprint_version, fingerprint, draft_id, operation_id, created_at, stage, account_id, snapshot, staged_at)
                    VALUES ('agent', '{key}', 0, 1, '{key}', '{draft}', '{operation}', 0, 'drafted', 'work', '{{}}', 0);
                INSERT INTO remote_draft_quarantine(account_id, account_incarnation, mailbox, provider_id, raw_sha256, raw, reason, created_at, last_seen_at)
                    SELECT 'work', token, 'Drafts', '1:9', '{key}', x'00', 'conflict', 0, 0 FROM account_incarnations;",
                key = "a".repeat(64), draft = "d".repeat(32), operation = "o".repeat(32))).unwrap();
            assert_eq!(objects(&old, LEFTOVERS).len(), LEFTOVERS.len(), "{:?}", objects(&old, LEFTOVERS));
        }
        let mut store = Store::open(&path).unwrap();
        assert!(objects(&store, LEFTOVERS).is_empty(), "{:?}", objects(&store, LEFTOVERS));
        assert_eq!(store.accounts().unwrap().len(), 1);
        assert_eq!(store.conn.query_row("SELECT count(*) FROM messages", [], |r| r.get::<_, i64>(0)).unwrap(), 1);
        // Adding an account no longer fires a trigger into a table that is gone.
        store.upsert_account(&account("personal", "maya@fastmail.com")).unwrap();
        assert!(store.remove_account("personal").unwrap());
        assert!(store.conn.prepare("PRAGMA foreign_key_check").unwrap().query([]).unwrap().next().unwrap().is_none());
        let check: String = store.conn.query_row("PRAGMA integrity_check", [], |r| r.get(0)).unwrap();
        assert_eq!(check, "ok");
        // No SQL left anywhere names them.
        let mut statement = store.conn.prepare("SELECT name FROM sqlite_master WHERE sql LIKE '%agent_requests%' OR sql LIKE '%incarnation%' OR sql LIKE '%remote_draft%'").unwrap();
        assert!(statement.query([]).unwrap().next().unwrap().is_none());
    }

    #[test]
    fn v50_asks_again_which_tab_important_mail_belongs_in_where_the_model_left_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("moss.db");
        {
            let mut old = historical_store(&path, 49);
            old.upsert_account(&account("work", "maya@example.test")).unwrap();
            for (id, subject) in [("1", "Stayed"), ("2", "Moved"), ("3", "List")] {
                ingest(&mut old, id, &mail().id(&format!("{id}@x")).subject(subject).build());
            }
            old.conn.execute_batch("
                UPDATE messages SET split = 'receipts' WHERE subject = 'Moved';
                UPDATE messages SET split = 'newsletters' WHERE subject = 'List';
                INSERT INTO ai_decisions (message_id, question, yes, source, acted, value, decided_at)
                    SELECT id, 'split', 0.9, 'model', subject = 'Moved', CASE subject WHEN 'Stayed' THEN 'important'
                        WHEN 'Moved' THEN 'receipts' ELSE 'newsletters' END, 0 FROM messages;
                INSERT INTO ai_decisions (message_id, question, yes, source, acted, value, decided_at)
                    SELECT id, 'needsReply', 0.9, 'model', 1, 'reply', 0 FROM messages WHERE subject = 'Stayed';").unwrap();
        }
        let store = Store::open(&path).unwrap();
        let decided: Vec<(String, String)> = store.conn
            .prepare("SELECT m.subject, d.question FROM ai_decisions d JOIN messages m ON m.id = d.message_id ORDER BY 1, 2").unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?))).unwrap().collect::<rusqlite::Result<_>>().unwrap();
        // Only the split the model left in Important goes, so it is asked again with the pitch
        // question. Its other answers, and splits it moved or took from a list, stay.
        let decided: Vec<(&str, &str)> = decided.iter().map(|(m, q)| (m.as_str(), q.as_str())).collect();
        assert_eq!(decided, [("List", "split"), ("Moved", "split"), ("Stayed", "needsReply")]);
    }
}
