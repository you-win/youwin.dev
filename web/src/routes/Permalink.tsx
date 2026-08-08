import { A, useParams } from "@solidjs/router";
import { createResource, createSignal, For, Show } from "solid-js";

import PostCard from "../components/PostCard";
import { api, type Post } from "../lib/api";

/**
 * The gutter for one nesting level, mirroring `indent` in
 * crates/server/src/public/view/post.rs.
 *
 * Written out rather than computed, because Tailwind scans the source for class
 * names and cannot see one built from a number. Kept in step with the server's
 * copy so this does not read as a different site from /preview, which renders
 * through the public templates.
 */
const INDENT = [
  "",
  "ml-3 border-l border-base-300 pl-2 sm:ml-4 sm:pl-3",
  "ml-6 border-l border-base-300 pl-2 sm:ml-8 sm:pl-3",
  "ml-9 border-l border-base-300 pl-2 sm:ml-12 sm:pl-3",
  "ml-12 border-l border-base-300 pl-2 sm:ml-16 sm:pl-3",
];

/** Past the last step a reply keeps its place in the order and stops moving right. */
const indent = (depth: number) => INDENT[Math.min(depth, INDENT.length - 1)];

export default function Permalink() {
  const params = useParams<{ id: string }>();
  const [thread, { mutate, refetch }] = createResource(
    () => params.id,
    (id) => api.show(id),
  );

  const [deleted, setDeleted] = createSignal(false);

  const replace = (updated: Post) =>
    mutate((current) =>
      current
        ? {
            post: current.post.id === updated.id ? updated : current.post,
            // Spread over the existing row rather than replacing it: an edit
            // cannot move a post in the tree, and `updated` comes back from
            // PATCH without a `depth` to move it with.
            thread: current.thread.map((post) =>
              post.id === updated.id ? { ...post, ...updated } : post,
            ),
          }
        : current,
    );

  const remove = (id: string) => {
    // Deleting the post you are looking at leaves nowhere to be.
    if (id === thread()?.post.id) {
      setDeleted(true);
      return;
    }
    // Anything else reshapes the tree — deleting a reply that has replies of
    // its own re-parents them to the top level — so ask the server what the
    // thread looks like now instead of guessing at it here.
    void refetch();
  };

  // A reply belongs under what it answered, which is a position only the
  // server's `nest` knows. One request, and no second implementation of the
  // tree living in the client. The resource keeps showing the old thread while
  // this is in flight, so nothing flashes.
  const reload = () => void refetch();

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
                  <div class={indent(post.depth)}>
                    <PostCard
                      post={post}
                      focused={post.id === loaded().post.id}
                      onChanged={replace}
                      onDeleted={remove}
                      onReply={reload}
                    />
                  </div>
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
