-- Frozen schema from main 4ad0d281c80e1c599877122b8a7eeab20cc53b5a. Synthetic upgrade fixture; do not regenerate from current migrations.

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


CREATE TABLE settings (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);


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


ALTER TABLE messages ADD COLUMN done_at INTEGER;


CREATE TABLE outbox (
    id         INTEGER PRIMARY KEY,
    account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    change     TEXT NOT NULL,
    created_at INTEGER NOT NULL
);
CREATE INDEX outbox_by_account ON outbox (account_id, id);


CREATE TABLE ai_decisions (
    message_id INTEGER NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
    question   TEXT NOT NULL,
    yes        REAL NOT NULL,
    acted      INTEGER NOT NULL,
    decided_at INTEGER NOT NULL,
    PRIMARY KEY (message_id, question)
);


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


-- Where a deleted message was when you deleted it, so undo puts it back there:
-- 'inbox' or 'done'. NULL for mail that isn't deleted.
ALTER TABLE messages ADD COLUMN deleted_from TEXT CHECK (deleted_from IN ('inbox', 'done'));




ALTER TABLE drafts ADD COLUMN signature_id TEXT;
ALTER TABLE drafts ADD COLUMN signature_chosen INTEGER NOT NULL DEFAULT 0;


-- IMAP $Forwarded: set by Moss or another mail app.
ALTER TABLE messages ADD COLUMN forwarded INTEGER NOT NULL DEFAULT 0;

-- A message you forwarded from Moss, and the sent copy that carried it.
CREATE TABLE forwards (
    message_id INTEGER NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
    sent_id    INTEGER NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
    PRIMARY KEY (message_id, sent_id)
);


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


ALTER TABLE drafts ADD COLUMN server_message_id TEXT;
ALTER TABLE drafts ADD COLUMN server_provider_id TEXT;
ALTER TABLE drafts ADD COLUMN pushed_at INTEGER;


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


ALTER TABLE threads ADD COLUMN moved_to TEXT
    CHECK (moved_to IN ('other', 'calendar', 'newsletters', 'receipts'));


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

PRAGMA user_version=24;
