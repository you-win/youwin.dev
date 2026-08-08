import { A, useParams } from "@solidjs/router";
import { createResource, createSignal, For, Show } from "solid-js";

import PostCard from "../components/PostCard";
import { api, type Post } from "../lib/api";

export default function Permalink() {
  const params = useParams<{ id: string }>();
  const [thread, { mutate }] = createResource(
    () => params.id,
    (id) => api.show(id),
  );

  const [deleted, setDeleted] = createSignal(false);

  const replace = (updated: Post) =>
    mutate((current) =>
      current
        ? {
            post: current.post.id === updated.id ? updated : current.post,
            thread: current.thread.map((post) =>
              post.id === updated.id ? updated : post,
            ),
          }
        : current,
    );

  const remove = (id: string) => {
    // Deleting the post you are looking at leaves nowhere to be; deleting a
    // reply just drops it from the thread.
    if (id === thread()?.post.id) {
      setDeleted(true);
      return;
    }
    mutate((current) =>
      current
        ? { ...current, thread: current.thread.filter((post) => post.id !== id) }
        : current,
    );
  };

  const append = (reply: Post) =>
    mutate((current) =>
      current ? { ...current, thread: [...current.thread, reply] } : current,
    );

  return (
    <div class="flex flex-col gap-3">
      <Show
        when={!deleted()}
        fallback={
          <div class="py-8 text-center">
            <p class="text-sm text-secondary">Deleted.</p>
            <A href="/" class="mt-2 inline-block text-sm">
              ← back to the feed
            </A>
          </div>
        }
      >
        <Show
          when={thread()}
          fallback={
            <p class="py-8 text-center text-sm text-secondary">
              {thread.error ? "Not found." : "Loading…"}
            </p>
          }
        >
          {(loaded) => (
            <>
              <For each={loaded().thread}>
                {(post) => (
                  <PostCard
                    post={post}
                    focused={post.id === loaded().post.id}
                    onChanged={replace}
                    onDeleted={remove}
                    onReply={append}
                  />
                )}
              </For>

              <A href="/" class="mt-4 text-sm">
                ← back to the feed
              </A>
            </>
          )}
        </Show>
      </Show>
    </div>
  );
}
