import { useSearchParams } from "@solidjs/router";
import { createEffect, createResource, createSignal, For, on, Show } from "solid-js";

import PostCard from "../components/PostCard";
import { api, type Post } from "../lib/api";

/**
 * Pause after the last keystroke before asking the server. Long enough that
 * typing a word is one request rather than five, short enough that it still
 * feels like the results are following you.
 */
const DEBOUNCE_MS = 250;

/**
 * Search across everything, drafts included.
 *
 * The query lives in the URL rather than in a signal alone, so a search is a
 * page you can go back to — the same property the public site's `/search` has,
 * and the reason a result found on a phone can be sent to a laptop.
 */
export default function Search() {
  const [params, setParams] = useSearchParams<{ q?: string }>();

  const initial = () => params.q ?? "";
  const [typed, setTyped] = createSignal(initial());
  // Trails `typed` by DEBOUNCE_MS. Kept separate so the input stays perfectly
  // responsive while the request it triggers does not.
  const [settled, setSettled] = createSignal(initial().trim());

  let timer: ReturnType<typeof setTimeout> | undefined;

  createEffect(
    on(typed, (value) => {
      clearTimeout(timer);
      timer = setTimeout(() => {
        const trimmed = value.trim();
        setSettled(trimmed);
        // `replace`, not push: otherwise every intermediate keystroke becomes a
        // history entry and Back has to be pressed once per letter.
        setParams({ q: trimmed || undefined }, { replace: true });
      }, DEBOUNCE_MS);
    }),
  );

  const [results, { mutate }] = createResource(
    // An empty query resolves to `false`, which createResource treats as "do not
    // fetch" — so clearing the box shows the prompt again rather than every post.
    () => settled() || false,
    (query: string) => api.search(query),
  );

  const replace = (updated: Post) =>
    mutate((current) =>
      current
        ? {
            ...current,
            posts: current.posts.map((post) =>
              post.id === updated.id ? updated : post,
            ),
          }
        : current,
    );

  const remove = (id: string) =>
    mutate((current) =>
      current
        ? { ...current, posts: current.posts.filter((post) => post.id !== id) }
        : current,
    );

  return (
    <div class="flex flex-col gap-4">
      <input
        type="search"
        class="input w-full border-base-300 bg-base-200"
        placeholder="Search everything, drafts included"
        aria-label="Search posts"
        autocomplete="off"
        spellcheck={false}
        value={typed()}
        autofocus
        onInput={(event) => setTyped(event.currentTarget.value)}
      />

      <Show when={settled()} fallback={<Prompt />}>
        <Show
          when={results()}
          fallback={<p class="text-sm text-secondary">Searching…</p>}
        >
          {(page) => (
            <Show
              when={page().posts.length > 0}
              fallback={
                <p class="py-8 text-center text-sm text-secondary">
                  Nothing matches “{settled()}”.
                </p>
              }
            >
              <For each={page().posts}>
                {(post) => (
                  <PostCard post={post} onChanged={replace} onDeleted={remove} />
                )}
              </For>
            </Show>
          )}
        </Show>
      </Show>
    </div>
  );
}

function Prompt() {
  return (
    <p class="py-8 text-center text-sm text-secondary">
      Whole words, in any order. Everything you have written is in here.
    </p>
  );
}
