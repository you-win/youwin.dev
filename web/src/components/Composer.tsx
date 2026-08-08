import { createSignal, Show } from "solid-js";

import type { Visibility } from "../lib/api";

/** Soft limit: the meter turns amber past this, but the post is still allowed. */
const SOFT_LIMIT = 500;

/** Hard limit, mirroring the server's. Exceeding it disables posting. */
const HARD_LIMIT = 4000;

interface Props {
  /** Resolves once the post is stored; rejects to leave the draft in place. */
  onSubmit: (body: string, visibility: Visibility) => Promise<void>;
  placeholder?: string;
  submitLabel?: string;
  /** Replies are always public — a draft reply to a published post is a muddle. */
  allowDraft?: boolean;
  initialBody?: string;
  autofocus?: boolean;
  onCancel?: () => void;
}

export default function Composer(props: Props) {
  const [body, setBody] = createSignal(props.initialBody ?? "");
  const [visibility, setVisibility] = createSignal<Visibility>("public");
  const [busy, setBusy] = createSignal(false);
  const [error, setError] = createSignal<string | null>(null);

  let textarea: HTMLTextAreaElement | undefined;

  const count = () => [...body()].length;
  const overSoft = () => count() > SOFT_LIMIT;
  const overHard = () => count() > HARD_LIMIT;
  const empty = () => body().trim().length === 0;

  // Grow to fit rather than scroll: a composer that scrolls hides the start of
  // what you are writing, which for a 300-character post is most of it.
  const resize = () => {
    if (!textarea) return;
    textarea.style.height = "auto";
    textarea.style.height = `${textarea.scrollHeight}px`;
  };

  const submit = async () => {
    if (empty() || overHard() || busy()) return;

    setBusy(true);
    setError(null);
    try {
      await props.onSubmit(body(), visibility());
      setBody("");
      setVisibility("public");
      queueMicrotask(resize);
    } catch (e) {
      // The text stays in the box. Losing a post to a failed request would be
      // unforgivable, and it is the one failure mode that actually costs
      // something irrecoverable.
      setError(e instanceof Error ? e.message : "Could not post.");
    } finally {
      setBusy(false);
    }
  };

  const onKeyDown = (event: KeyboardEvent) => {
    if ((event.metaKey || event.ctrlKey) && event.key === "Enter") {
      event.preventDefault();
      void submit();
    }
    if (event.key === "Escape" && props.onCancel) {
      event.preventDefault();
      props.onCancel();
    }
  };

  return (
    <div class="rounded-box border border-base-300 bg-base-200 p-4">
      <textarea
        ref={(el) => {
          textarea = el;
          if (props.autofocus) queueMicrotask(() => el.focus());
          queueMicrotask(resize);
        }}
        class="w-full resize-none bg-transparent leading-relaxed outline-none placeholder:text-base-content/40"
        rows="3"
        placeholder={props.placeholder ?? "What's on your mind?"}
        value={body()}
        disabled={busy()}
        onInput={(event) => {
          setBody(event.currentTarget.value);
          resize();
        }}
        onKeyDown={onKeyDown}
      />

      <Show when={error()}>
        {(message) => (
          <p class="mt-2 text-sm text-error" role="alert">
            {message()}
          </p>
        )}
      </Show>

      <div class="mt-3 flex flex-wrap items-center justify-between gap-3">
        <div class="flex items-center gap-3">
          <span
            class="text-sm tabular-nums"
            classList={{
              "text-secondary": !overSoft(),
              "text-warning": overSoft() && !overHard(),
              "text-error": overHard(),
            }}
          >
            {count()} / {SOFT_LIMIT}
          </span>

          <Show when={props.allowDraft !== false}>
            <select
              class="select select-sm border-base-300 bg-base-100"
              value={visibility()}
              disabled={busy()}
              onChange={(event) =>
                setVisibility(event.currentTarget.value as Visibility)
              }
            >
              <option value="public">Public</option>
              <option value="unlisted">Unlisted</option>
              <option value="draft">Draft</option>
            </select>
          </Show>
        </div>

        <div class="flex items-center gap-2">
          <Show when={props.onCancel}>
            <button
              type="button"
              class="btn btn-ghost btn-sm"
              onClick={props.onCancel}
              disabled={busy()}
            >
              Cancel
            </button>
          </Show>
          <button
            type="button"
            class="btn btn-primary btn-sm"
            disabled={empty() || overHard() || busy()}
            onClick={() => void submit()}
          >
            {busy() ? "Posting…" : (props.submitLabel ?? "Post")}
          </button>
        </div>
      </div>

      <p class="mt-2 text-xs text-base-content/40">
        Markdown. {navigator.platform.includes("Mac") ? "⌘" : "Ctrl"}+Enter to
        post.
      </p>
    </div>
  );
}
