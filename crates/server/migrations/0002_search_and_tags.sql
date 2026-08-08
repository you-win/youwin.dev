-- M5: full-text search over the plaintext projection, and hashtags.

-- External content: the index stores terms, `posts` stores the text. Without
-- `content=` FTS5 would keep a second copy of every post, and the two copies
-- could drift. `content_rowid=id` ties an index row to a post's rowid.
--
-- `porter` wraps `unicode61` with an English stemmer, so "cat" finds "cats" and
-- "running" finds "run". Worth stating plainly: it is English-only. Search over
-- posts in another language degrades to exact-token matching, which is what an
-- unstemmed index would have given anyway — so this is strictly better, never
-- worse. `remove_diacritics 2` folds accents without mangling the two Turkish
-- dotted-i cases the default mode gets wrong.
CREATE VIRTUAL TABLE posts_fts USING fts5(
  body_text,
  content='posts',
  content_rowid='id',
  tokenize='porter unicode61 remove_diacritics 2'
);

-- Backfill. Runs before the triggers exist, though the order is immaterial:
-- these triggers watch `posts`, and this statement writes to `posts_fts`.
INSERT INTO posts_fts(rowid, body_text) SELECT id, body_text FROM posts;

CREATE TRIGGER posts_fts_ai AFTER INSERT ON posts BEGIN
  INSERT INTO posts_fts(rowid, body_text) VALUES (new.id, new.body_text);
END;

-- An external-content index cannot re-read a row that is already gone, so a
-- deletion has to hand back the old text for the terms to be un-indexed. That is
-- what the 'delete' command is for. Nothing in this application hard-deletes a
-- post, but a trigger that only half-works is worse than no trigger at all.
CREATE TRIGGER posts_fts_ad AFTER DELETE ON posts BEGIN
  INSERT INTO posts_fts(posts_fts, rowid, body_text)
    VALUES ('delete', old.id, old.body_text);
END;

-- `OF body_text` narrows this to the one column the index cares about. Soft
-- deletes and visibility flips both UPDATE `posts` on every edit; without the
-- column list each of them would tear down and rebuild an index entry that did
-- not change.
CREATE TRIGGER posts_fts_au AFTER UPDATE OF body_text ON posts BEGIN
  INSERT INTO posts_fts(posts_fts, rowid, body_text)
    VALUES ('delete', old.id, old.body_text);
  INSERT INTO posts_fts(rowid, body_text) VALUES (new.id, new.body_text);
END;

-- Soft-deleted and draft posts stay in the index; every search joins `posts` and
-- filters there. Keeping the index a plain mirror of the text means a visibility
-- change is one UPDATE rather than an index edit that could fail halfway.

-- Hashtags. `tag` is the lowercased form and the thing matched on; `display`
-- keeps the first casing written, so a tag page can title itself "#TypeScript"
-- rather than "#typescript".
--
-- Lowercasing happens in Rust, not via COLLATE NOCASE: SQLite's NOCASE folds
-- ASCII only, so "#Café" and "#café" would be two different tags.
CREATE TABLE tags (
  id      INTEGER PRIMARY KEY,
  tag     TEXT NOT NULL UNIQUE,
  display TEXT NOT NULL
);

CREATE TABLE post_tags (
  post_id INTEGER NOT NULL REFERENCES posts(id) ON DELETE CASCADE,
  tag_id  INTEGER NOT NULL REFERENCES tags(id) ON DELETE CASCADE,
  PRIMARY KEY (post_id, tag_id)
) WITHOUT ROWID;

-- The tag page's access path: every post carrying a tag, newest first. The PK
-- above already covers the other direction (a post's tags).
CREATE INDEX idx_post_tags_tag ON post_tags (tag_id, post_id);
