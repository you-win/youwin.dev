-- M9: a key that makes "post this" safe to retry.
--
-- The composer can now queue a post written offline and flush it when the
-- connection comes back. A flush is a retry, and a retry over an unreliable
-- link has no way to tell "the request never arrived" from "the response never
-- came back" — so without this, the honest client behaviour is either to risk
-- posting twice or to risk losing the post. Neither is acceptable for the one
-- thing this application exists to do.
--
-- The client generates the key once, when the post is queued, and sends the same
-- one on every attempt. The first attempt to reach the server wins; every later
-- one gets that same post back instead of a second copy.
--
-- Nullable, because almost nothing uses it: a post made online carries no key,
-- and every post written before this migration has none. SQLite allows any
-- number of NULLs in a unique index, which is exactly the semantics wanted —
-- "no key" is not a value that can collide.
ALTER TABLE posts ADD COLUMN idempotency_key TEXT;

-- A partial unique index rather than a UNIQUE column constraint, because SQLite
-- cannot add one with ALTER TABLE at all — `ADD COLUMN` refuses UNIQUE outright.
-- The WHERE clause is not an optimization: it keeps the index off the rows that
-- have no key, which is all of them.
CREATE UNIQUE INDEX idx_posts_idempotency
    ON posts (idempotency_key)
 WHERE idempotency_key IS NOT NULL;
