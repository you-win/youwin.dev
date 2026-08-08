-- M7: mood becomes a field on the post rather than a hashtag inside it.
--
-- The familiar read mood from `#tired`, `#excited` and friends in the body. That
-- worked, but it was only discoverable if you already knew the seven names — so
-- in practice it was a feature you had to remember rather than one you could
-- see. It is now a picker in the composer, which means it needs somewhere to
-- live.
--
-- Nullable on purpose. NULL is "I did not say", and the familiar falls back to
-- inferring one from the text; a stored value is "I did say", and nothing
-- overrides it. An explicit 'neutral' is therefore different from NULL: it means
-- "nothing to report", and it turns inference off for that post.

ALTER TABLE posts ADD COLUMN mood TEXT
  CHECK (mood IS NULL OR mood IN
    ('content','contemplative','tired','excited','melancholy','chaos','neutral'));

-- Backfill from the hashtags that used to carry it.
--
-- Read from `post_tags` rather than by pattern-matching `body_text`: those rows
-- were written by the same pass that decided what a hashtag was, so they are the
-- extraction rules rather than an approximation of them.
--
-- The tag itself is deliberately left in place. It is a perfectly good tag, /t/tired
-- keeps working, and rewriting bodies that were already published to remove a
-- word is not something a migration should do quietly.
--
-- The names are spelled out here rather than shared with the Rust enum, which is
-- the point of a migration: this is a snapshot of what the old scheme meant, and
-- it must not change meaning later because someone added a mood.
--
-- The ORDER BY only matters for a post that somehow carried two mood tags. It
-- matches the order the old `detect` scanned in, so a post that was ambiguous
-- before lands on the same answer it was already showing.
UPDATE posts SET mood = (
  SELECT t.tag
    FROM post_tags pt
    JOIN tags t ON t.id = pt.tag_id
   WHERE pt.post_id = posts.id
     AND t.tag IN
       ('content','contemplative','tired','excited','melancholy','chaos','neutral')
   ORDER BY CASE t.tag
              WHEN 'content'       THEN 0
              WHEN 'contemplative' THEN 1
              WHEN 'tired'         THEN 2
              WHEN 'excited'       THEN 3
              WHEN 'melancholy'    THEN 4
              WHEN 'chaos'         THEN 5
              ELSE 6
            END
   LIMIT 1
);
