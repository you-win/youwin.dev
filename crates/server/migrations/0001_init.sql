-- Initial schema. See DESIGN.md "Data model" for why each column exists.
--
-- There is no `users` table on purpose: there is one user, and the password hash
-- lives in the environment. A table modelling a user would be a table with one
-- row and no purpose.

CREATE TABLE posts (
  id          INTEGER PRIMARY KEY,           -- rowid; monotonic, used for ordering
  public_id   TEXT    NOT NULL UNIQUE,       -- 12 random bytes, base64url (16 chars)
  parent_id   INTEGER REFERENCES posts(id) ON DELETE CASCADE,
  root_id     INTEGER NOT NULL,              -- thread head; equals id for roots
  body        TEXT    NOT NULL,              -- markdown source — the authority
  body_html   TEXT    NOT NULL,              -- rendered + sanitized, cached at write
  body_text   TEXT    NOT NULL,              -- plaintext, for OG descriptions and search
  visibility  TEXT    NOT NULL DEFAULT 'public'
                CHECK (visibility IN ('public','unlisted','draft')),
  created_at  INTEGER NOT NULL,              -- unix millis, UTC
  updated_at  INTEGER NOT NULL,
  edited_at   INTEGER,                       -- null until the body changes post-publish
  deleted_at  INTEGER                        -- soft delete; rows are never removed
);

-- The partial predicates match the WHERE clause every read carries, so the
-- `deleted_at IS NULL` filter costs nothing.
CREATE INDEX idx_posts_feed   ON posts (created_at DESC, id DESC) WHERE deleted_at IS NULL;
CREATE INDEX idx_posts_root   ON posts (root_id, created_at)      WHERE deleted_at IS NULL;
CREATE INDEX idx_posts_parent ON posts (parent_id)                WHERE deleted_at IS NULL;

CREATE TABLE sessions (
  token_hash   BLOB    PRIMARY KEY,          -- SHA-256 of the cookie value, never the value
  created_at   INTEGER NOT NULL,
  expires_at   INTEGER NOT NULL,
  last_seen_at INTEGER NOT NULL,
  user_agent   TEXT,
  ip           TEXT
);

CREATE INDEX idx_sessions_expiry ON sessions (expires_at);
