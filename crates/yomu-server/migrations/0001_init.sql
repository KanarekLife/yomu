-- Server-wide revision counter used as the sync cursor.
CREATE SEQUENCE rev_seq;

CREATE TABLE users (
    id            BIGSERIAL PRIMARY KEY,
    username      TEXT NOT NULL UNIQUE CHECK (username = lower(username) AND username <> ''),
    password_hash TEXT NOT NULL,
    is_admin      BOOLEAN NOT NULL DEFAULT FALSE,
    disabled      BOOLEAN NOT NULL DEFAULT FALSE,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE tokens (
    token_hash   BYTEA PRIMARY KEY,
    user_id      BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    device_name  TEXT NOT NULL DEFAULT '',
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_used_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX tokens_user_idx ON tokens (user_id);

CREATE TABLE books (
    user_id      BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    key          TEXT NOT NULL,
    title        TEXT NOT NULL DEFAULT '',
    author       TEXT NOT NULL DEFAULT '',
    position     BIGINT NOT NULL DEFAULT 0,
    last_read_at BIGINT,
    added_at     BIGINT NOT NULL DEFAULT 0,
    deleted      BOOLEAN NOT NULL DEFAULT FALSE,
    updated_at   BIGINT NOT NULL,
    rev          BIGINT NOT NULL,
    PRIMARY KEY (user_id, key)
);
CREATE INDEX books_rev_idx ON books (user_id, rev);

CREATE TABLE settings (
    user_id    BIGINT PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
    data       JSONB NOT NULL,
    updated_at BIGINT NOT NULL,
    rev        BIGINT NOT NULL
);

CREATE TABLE translations (
    user_id    BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    key        TEXT NOT NULL,
    idx        BIGINT NOT NULL,
    lang       TEXT NOT NULL,
    text       TEXT NOT NULL,
    model      TEXT NOT NULL DEFAULT '',
    updated_at BIGINT NOT NULL,
    rev        BIGINT NOT NULL,
    PRIMARY KEY (user_id, key, idx, lang)
);
CREATE INDEX translations_rev_idx ON translations (user_id, rev);

CREATE TABLE files (
    user_id     BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    key         TEXT NOT NULL,
    data        BYTEA NOT NULL,
    size        BIGINT NOT NULL,
    uploaded_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (user_id, key)
);

CREATE TABLE admin_sessions (
    id_hash    BYTEA PRIMARY KEY,
    user_id    BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    expires_at TIMESTAMPTZ NOT NULL
);
CREATE INDEX admin_sessions_user_idx ON admin_sessions (user_id);
