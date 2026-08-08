import { createResource, For, Show } from "solid-js";

import PostCard from "../components/PostCard";
import { api, type Post } from "../lib/api";

export default function Drafts() {
  const [drafts, { mutate }] = createResource(() => api.drafts());

  const replace = (updated: Post) =>
    mutate((current) =>
      current
        ? {
            ...current,
            // A published draft is no longer a draft, so it leaves this list
            // rather than sitting here mislabelled.
            posts:
              updated.visibility === "draft"
                ? current.posts.map((post) =>
                    post.id === updated.id ? updated : post,
                  )
                : current.posts.filter((post) => post.id !== updated.id),
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
      <h1 class="text-lg font-medium">Drafts</h1>

      <Show
        when={drafts()}
        fallback={<p class="text-sm text-secondary">Loading…</p>}
      >
        {(page) => (
          <Show
            when={page().posts.length > 0}
            fallback={
              <p class="py-8 text-center text-sm text-secondary">
                No drafts. Everything you have written is published.
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
    </div>
  );
}
