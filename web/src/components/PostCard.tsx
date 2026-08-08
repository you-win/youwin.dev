import { A } from "@solidjs/router";
import { createSignal, Show } from "solid-js";

import {
  api,
  previewUrl,
  share,
  type Mood,
  type Post,
  type Visibility,
} from "../lib/api";
import { absolute, relative } from "../lib/format";
import Composer from "./Composer";

interface Props {
  post: Post;
  /** Set when the post is still in flight, before the server has rendered it. */
  pending?: boolean;
  /** Marks the post whose permalink you followed, inside a thread. */
  focused?: boolean;
  onChanged?: (post: Post) => void;
  onDeleted?: (id: string, count: number) => void;
  onReply?: (post: Post) => void;
}

const VISIBILITY_LABEL: Record<Visibility, string | null> = {
  public: null,
  unlisted: "unlisted",
  draft: "draft",
};

export default function PostCard(props: Props) {
  const [editing, setEditing] = createSignal(false);
  const [replying, setReplying] = createSignal(false);
  const [confirming, setConfirming] = createSignal(false);
  const [busy, setBusy] = createSignal(false);
  const [shared, setShared] = createSignal<string | null>(null);

  const badge = () => VISIBILITY_LABEL[props.post.visibility];

  // Sent on every save, including as an explicit null, so clearing a mood back
  // to "did not say" is expressible — omitting the key would mean "leave it".
  const save = async (body: string, _visibility: Visibility, mood: Mood | null) => {
    const updated = await api.update(props.post.id, { body, mood });
    props.onChanged?.(updated);
    setEditing(false);
  };

  const publish = async () => {
    setBusy(true);
    try {
      props.onChanged?.(
        await api.update(props.post.id, { visibility: "public" }),
      );
    } finally {
      setBusy(false);
    }
  };

  const destroy = async () => {
    setBusy(true);
    try {
      const { deleted } = await api.destroy(props.post.id);
      props.onDeleted?.(props.post.id, deleted);
    } finally {
      setBusy(false);
      setConfirming(false);
    }
  };

  const reply = async (
    body: string,
    _visibility: Visibility,
    mood: Mood | null,
  ) => {
    const created = await api.create(body, "public", mood, props.post.id);
    props.onReply?.(created);
    setReplying(false);
  };

  return (
    <article
      class="rounded-box border p-4 transition-opacity"
      classList={{
        "border-primary/40 bg-base-200": props.focused === true,
        "border-base-300 bg-base-200": props.focused !== true,
        "opacity-50": props.pending === true || busy(),
      }}
    >
      <header class="mb-2 flex items-center gap-2 text-sm text-secondary">
        <A
          href={`/p/${props.post.id}`}
          class="no-underline hover:underline"
          title={absolute(props.post.created_at)}
        >
          {relative(props.post.created_at)}
        </A>

        <Show when={props.post.edited_at}>
          <span title="Edited after publishing">· edited</span>
        </Show>

        {/* Shown here and nowhere on youwin.dev. Without it the mood is
            invisible until you open the editor, which is most of what made the
            hashtag version hard to keep track of. */}
        <Show when={props.post.mood}>
          {(mood) => (
            <span title="Mood — feeds the familiar, not shown publicly">
              · {mood()}
            </span>
          )}
        </Show>

        <Show when={badge()}>
          {(label) => (
            <span class="badge badge-sm border-base-300 bg-base-300 text-accent">
              {label()}
            </span>
          )}
        </Show>

        <Show when={props.pending}>
          <span class="text-base-content/40">· posting…</span>
        </Show>
      </header>

      <Show
        when={!editing()}
        fallback={
          <Composer
            initialBody={props.post.body}
            initialMood={props.post.mood}
            submitLabel="Save"
            allowDraft={false}
            autofocus
            onSubmit={save}
            onCancel={() => setEditing(false)}
          />
        }
      >
        <Show
          when={!props.pending}
          fallback={
            // Rendering happens server-side, so an in-flight post has no HTML
            // yet. Showing the raw text keeps the layout stable rather than
            // flashing an empty card.
            <div class="post-body whitespace-pre-wrap">{props.post.body}</div>
          }
        >
          {/* Server-sanitized at write time; see render::markdown. */}
          <div class="post-body" innerHTML={props.post.body_html} />
        </Show>
      </Show>

      <Show when={!editing() && !props.pending}>
        <footer class="mt-3 flex flex-wrap items-center gap-1 text-sm">
          <Show when={props.post.reply_count > 0}>
            <A
              href={`/p/${props.post.id}`}
              class="btn btn-ghost btn-xs text-secondary"
            >
              {props.post.reply_count}{" "}
              {props.post.reply_count === 1 ? "reply" : "replies"}
            </A>
          </Show>

          <Show when={props.onReply}>
            <button
              type="button"
              class="btn btn-ghost btn-xs"
              onClick={() => setReplying((was) => !was)}
            >
              Reply
            </button>
          </Show>

          <button
            type="button"
            class="btn btn-ghost btn-xs"
            onClick={() => setEditing(true)}
          >
            Edit
          </button>

          <a
            class="btn btn-ghost btn-xs"
            href={previewUrl(props.post.id)}
            target="_blank"
            rel="noopener"
          >
            Preview
          </a>

          {/* A draft has no public URL yet, so there is nothing to share. */}
          <Show when={props.post.visibility !== "draft"}>
            <button
              type="button"
              class="btn btn-ghost btn-xs"
              onClick={async () => {
                const outcome = await share(props.post.id);
                if (outcome === "shared") return;
                // A share sheet is self-evident; a silent clipboard write is not.
                setShared(outcome === "copied" ? "Link copied" : "Could not share");
                setTimeout(() => setShared(null), 2000);
              }}
            >
              {shared() ?? "Share"}
            </button>
          </Show>

          <Show when={props.post.visibility === "draft"}>
            <button
              type="button"
              class="btn btn-ghost btn-xs text-primary"
              onClick={() => void publish()}
            >
              Publish
            </button>
          </Show>

          <Show
            when={!confirming()}
            fallback={
              <span class="flex items-center gap-1">
                <span class="text-error">
                  {props.post.reply_count > 0 && !props.post.is_reply
                    ? `Delete thread (${props.post.reply_count + 1} posts)?`
                    : "Delete?"}
                </span>
                <button
                  type="button"
                  class="btn btn-error btn-xs"
                  onClick={() => void destroy()}
                >
                  Yes
                </button>
                <button
                  type="button"
                  class="btn btn-ghost btn-xs"
                  onClick={() => setConfirming(false)}
                >
                  No
                </button>
              </span>
            }
          >
            <button
              type="button"
              class="btn btn-ghost btn-xs text-error/70 hover:text-error"
              onClick={() => setConfirming(true)}
            >
              Delete
            </button>
          </Show>
        </footer>
      </Show>

      <Show when={replying()}>
        <div class="mt-3">
          <Composer
            placeholder="Reply…"
            submitLabel="Reply"
            allowDraft={false}
            autofocus
            onSubmit={reply}
            onCancel={() => setReplying(false)}
          />
        </div>
      </Show>
    </article>
  );
}
