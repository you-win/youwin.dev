import { createSignal, For, onCleanup, onMount, Show } from "solid-js";

import Composer from "../components/Composer";
import Familiar, { type Draft } from "../components/Familiar";
import PostCard from "../components/PostCard";
import {
  api,
  NetworkError,
  type Mood,
  type Post,
  type Visibility,
} from "../lib/api";
import {
  discard,
  enqueue,
  onFlushed,
  queued,
  rejected,
  type Queued,
} from "../lib/outbox";

/** A post that exists locally but has not come back from the server yet. */
interface Pending {
  key: number;
  post: Post;
}

let nextKey = 0;

/**
 * A queued post, shaped like one so it can use the same card.
 *
 * Everything derived is a placeholder — there is no id, no rendered HTML and no
 * reply count until the server has seen it — and `pending` is what stops the
 * card offering actions that would need any of them.
 */
function asPost(item: Queued): Post {
  return {
    id: `queued-${item.key}`,
    body: item.body,
    body_html: "",
    visibility: item.visibility,
    mood: item.mood,
    is_reply: item.parentId !== undefined,
    reply_count: 0,
    created_at: item.queuedAt,
    edited_at: null,
  };
}

export default function Feed() {
  const [posts, setPosts] = createSignal<Post[]>([]);
  const [pending, setPending] = createSignal<Pending[]>([]);
  const [cursor, setCursor] = createSignal<string | null>(null);
  const [exhausted, setExhausted] = createSignal(false);
  const [loading, setLoading] = createSignal(false);
  const [error, setError] = createSignal<string | null>(null);
  const [draft, setDraft] = createSignal<Draft | null>(null);
  /** Bumped after every write, so the familiar refetches what it is now. */
  const [revision, setRevision] = createSignal(0);
  /** Which rejected post's text was just copied, for a moment of feedback. */
  const [copied, setCopied] = createSignal<string | null>(null);

  let sentinel: HTMLDivElement | undefined;

  const loadMore = async () => {
    if (loading() || exhausted()) return;

    setLoading(true);
    setError(null);
    try {
      const page = await api.feed(cursor());
      setPosts((existing) => [...existing, ...page.posts]);
      setCursor(page.next);
      if (!page.next) setExhausted(true);
    } catch (e) {
      setError(e instanceof Error ? e.message : "Could not load posts.");
    } finally {
      setLoading(false);
    }
  };

  onMount(() => {
    void loadMore();

    // A post that left the outbox belongs at the top of the feed like any
    // other. Without this it would not appear until the next reload, which on an
    // app you leave open is a long time to wonder whether it sent.
    onCleanup(
      onFlushed((post) => {
        setPosts((existing) =>
          existing.some((p) => p.id === post.id) ? existing : [post, ...existing],
        );
        setRevision((n) => n + 1);
      }),
    );

    // Infinite scroll is fine *here*: this surface is never crawled and never
    // linked into. The public archive paginates by link instead.
    const observer = new IntersectionObserver((entries) => {
      if (entries.some((entry) => entry.isIntersecting)) void loadMore();
    });
    if (sentinel) observer.observe(sentinel);
    onCleanup(() => observer.disconnect());
  });

  const submit = async (
    body: string,
    visibility: Visibility,
    mood: Mood | null,
  ) => {
    // Show it immediately. The server owns rendering, so the optimistic card
    // displays the raw text until the real one arrives — honest about what it
    // is, and the layout does not jump.
    const key = nextKey++;
    const optimistic: Post = {
      id: `pending-${key}`,
      body,
      body_html: "",
      visibility,
      mood,
      is_reply: false,
      reply_count: 0,
      created_at: Date.now(),
      edited_at: null,
    };
    setPending((current) => [{ key, post: optimistic }, ...current]);

    try {
      const created = await api.create(body, visibility, mood);
      setPosts((existing) => [created, ...existing]);
      setRevision((n) => n + 1);
    } catch (e) {
      // There is no server to have refused this, so it is not the composer's
      // problem. Queue it and return normally: the box clears, the post moves
      // to the outbox, and it goes out when there is a connection.
      if (e instanceof NetworkError) {
        enqueue(body, visibility, mood);
        return;
      }
      // Anything the server *did* answer stays the composer's business, which
      // keeps the text in the box and says why.
      throw e;
    } finally {
      // Rolled back on failure as well as success — the Composer keeps the text
      // and surfaces the error, so nothing is lost.
      setPending((current) => current.filter((item) => item.key !== key));
    }
  };

  // Both bump the revision: an edit can change a mood and a delete changes the
  // count, and either moves the pet. A reply-count bump does not, but it arrives
  // through the same callback and one wasted fetch on this host is nothing.
  const replace = (updated: Post) => {
    setPosts((existing) =>
      existing.map((post) => (post.id === updated.id ? updated : post)),
    );
    setRevision((n) => n + 1);
  };

  const remove = (id: string) => {
    setPosts((existing) => existing.filter((post) => post.id !== id));
    setRevision((n) => n + 1);
  };

  return (
    <div class="flex flex-col gap-4">
      <Familiar draft={draft()} revision={revision()} />

      <Composer onSubmit={submit} onDraftChange={setDraft} draftKey="feed" />

      {/* A post the server refused. It keeps its text in full, because the
          alternative is deleting somebody's writing on their behalf over a 422.

          Copy rather than "put it back in the composer": the composer restores
          its own draft at construction, so refilling it means forcing a remount
          — which would throw away whatever is being typed right now to recover
          something that is already on screen and selectable. */}
      <For each={rejected()}>
        {(item) => (
          <div class="rounded-box border border-error/40 bg-base-200 p-4">
            <p class="mb-2 text-sm text-error">
              This could not be posted: {item.error}
            </p>
            <div class="post-body whitespace-pre-wrap">{item.body}</div>
            <div class="mt-3 flex gap-2">
              <button
                type="button"
                class="btn btn-ghost btn-xs"
                onClick={async () => {
                  try {
                    await navigator.clipboard.writeText(item.body);
                    setCopied(item.key);
                    setTimeout(() => setCopied(null), 2000);
                  } catch {
                    // The text is on screen either way; nothing is lost.
                  }
                }}
              >
                {copied() === item.key ? "Copied" : "Copy text"}
              </button>
              <button
                type="button"
                class="btn btn-ghost btn-xs text-error/70 hover:text-error"
                onClick={() => discard(item.key)}
              >
                Discard
              </button>
            </div>
          </div>
        )}
      </For>

      <For each={queued()}>
        {(item) => (
          <PostCard
            post={asPost(item)}
            pending
            pendingLabel="waiting for a connection"
          />
        )}
      </For>

      <For each={pending()}>
        {(item) => <PostCard post={item.post} pending />}
      </For>

      <For each={posts()}>
        {(post) => (
          <PostCard
            post={post}
            onChanged={replace}
            onDeleted={remove}
            // The feed lists roots only, so a new reply does not appear here —
            // it just bumps the parent's count, without a refetch.
            onReply={() =>
              replace({ ...post, reply_count: post.reply_count + 1 })
            }
          />
        )}
      </For>

      <Show when={error()}>
        {(message) => (
          <div class="rounded-box border border-error/40 p-4 text-sm text-error">
            {message()}{" "}
            <button
              type="button"
              class="btn btn-ghost btn-xs"
              onClick={() => void loadMore()}
            >
              Retry
            </button>
          </div>
        )}
      </Show>

      <Show when={loading()}>
        <p class="py-4 text-center text-sm text-secondary">Loading…</p>
      </Show>

      <Show
        when={
          exhausted() &&
          posts().length === 0 &&
          queued().length === 0 &&
          !loading()
        }
      >
        <p class="py-8 text-center text-sm text-secondary">
          Nothing here yet. Write something.
        </p>
      </Show>

      <div ref={sentinel} class="h-px" />
    </div>
  );
}
