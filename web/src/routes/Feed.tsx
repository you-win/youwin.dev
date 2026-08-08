import { createSignal, For, onCleanup, onMount, Show } from "solid-js";

import Composer from "../components/Composer";
import PostCard from "../components/PostCard";
import { api, type Post, type Visibility } from "../lib/api";

/** A post that exists locally but has not come back from the server yet. */
interface Pending {
  key: number;
  post: Post;
}

let nextKey = 0;

export default function Feed() {
  const [posts, setPosts] = createSignal<Post[]>([]);
  const [pending, setPending] = createSignal<Pending[]>([]);
  const [cursor, setCursor] = createSignal<string | null>(null);
  const [exhausted, setExhausted] = createSignal(false);
  const [loading, setLoading] = createSignal(false);
  const [error, setError] = createSignal<string | null>(null);

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

    // Infinite scroll is fine *here*: this surface is never crawled and never
    // linked into. The public archive paginates by link instead.
    const observer = new IntersectionObserver((entries) => {
      if (entries.some((entry) => entry.isIntersecting)) void loadMore();
    });
    if (sentinel) observer.observe(sentinel);
    onCleanup(() => observer.disconnect());
  });

  const submit = async (body: string, visibility: Visibility) => {
    // Show it immediately. The server owns rendering, so the optimistic card
    // displays the raw text until the real one arrives — honest about what it
    // is, and the layout does not jump.
    const key = nextKey++;
    const optimistic: Post = {
      id: `pending-${key}`,
      body,
      body_html: "",
      visibility,
      is_reply: false,
      reply_count: 0,
      created_at: Date.now(),
      edited_at: null,
    };
    setPending((current) => [{ key, post: optimistic }, ...current]);

    try {
      const created = await api.create(body, visibility);
      setPosts((existing) => [created, ...existing]);
    } finally {
      // Rolled back on failure as well as success — the Composer keeps the text
      // and surfaces the error, so nothing is lost.
      setPending((current) => current.filter((item) => item.key !== key));
    }
  };

  const replace = (updated: Post) =>
    setPosts((existing) =>
      existing.map((post) => (post.id === updated.id ? updated : post)),
    );

  const remove = (id: string) =>
    setPosts((existing) => existing.filter((post) => post.id !== id));

  return (
    <div class="flex flex-col gap-4">
      <Composer onSubmit={submit} />

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

      <Show when={exhausted() && posts().length === 0 && !loading()}>
        <p class="py-8 text-center text-sm text-secondary">
          Nothing here yet. Write something.
        </p>
      </Show>

      <div ref={sentinel} class="h-px" />
    </div>
  );
}
