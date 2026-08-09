import { createEffect, createSignal, For, Show } from "solid-js";

import { MOODS, type Mood, type Visibility } from "../lib/api";
import type { Draft } from "./Familiar";

/** Soft limit: the meter turns amber past this, but the post is still allowed. */
const SOFT_LIMIT = 500;

/** Hard limit, mirroring the server's. Exceeding it disables posting. */
const HARD_LIMIT = 4000;

/**
 * Sentence case for the picker, since it sits beside "Public"/"Unlisted".
 *
 * Spelled out rather than capitalized in CSS: `text-transform` on `<option>` is
 * ignored by several browsers, and this is seven words.
 */
const MOOD_LABEL: Record<Mood, string> = {
  content: "Content",
  contemplative: "Contemplative",
  tired: "Tired",
  excited: "Excited",
  melancholy: "Melancholy",
  chaos: "Chaos",
  neutral: "Neutral",
};

interface Props {
  /** Resolves once the post is stored; rejects to leave the draft in place. */
  onSubmit: (
    body: string,
    visibility: Visibility,
    mood: Mood | null,
  ) => Promise<void>;
  placeholder?: string;
  submitLabel?: string;
  /** Replies are always public — a draft reply to a published post is a muddle. */
  allowDraft?: boolean;
  initialBody?: string;
  initialMood?: Mood | null;
  autofocus?: boolean;
  onCancel?: () => void;
  /**
   * Reports the contents on every change, for the familiar to react to.
   *
   * Published rather than rendered here: the pet is its own card above the
   * composer, mirroring where it sits on the public feed, and a composer that
   * knew about it could not also be the reply and edit box.
   */
  onDraftChange?: (draft: Draft) => void;
}

export default function Composer(props: Props) {
  const [body, setBody] = createSignal(props.initialBody ?? "");
  const [visibility, setVisibility] = createSignal<Visibility>("public");
  const [mood, setMood] = createSignal<Mood | null>(props.initialMood ?? null);
  const [busy, setBusy] = createSignal(false);
  const [error, setError] = createSignal<string | null>(null);

  let textarea: HTMLTextAreaElement | undefined;

  // Fires on any of the three, including the reset after a successful post,
  // which is what clears the familiar's preview rather than leaving it showing a
  // draft that has already been published.
  createEffect(() =>
    props.onDraftChange?.({
      body: body(),
      visibility: visibility(),
      mood: mood(),
    }),
  );

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
      await props.onSubmit(body(), visibility(), mood());
      setBody("");
      setVisibility("public");
      setMood(null);
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
        {/* Wraps for the same reason the row above it does. Everything here fits
            on one line down to 375px, but only just — the safety valve is what
            stops a longer mood name or a larger font from being absorbed by
            squashing the controls instead. */}
        <div class="flex flex-wrap items-center gap-3">
          <span
            class="shrink-0 whitespace-nowrap text-sm tabular-nums"
            classList={{
              "text-secondary": !overSoft(),
              "text-warning": overSoft() && !overHard(),
              "text-error": overHard(),
            }}
          >
            {count()} / {SOFT_LIMIT}
          </span>

          {/* `w-auto!` because daisyUI's `.select` is `width: 100%`, which in a
              shrink-to-fit flex row settles at a width narrower than the option
              it is displaying — "Contemplative" needed 103px and had 48px, and
              even the "Mood…" placeholder was clipped. Sizing to the selected
              option costs a small width change when one is picked, which beats
              a control that cannot show what it is set to. The `!` is the point
              of the important modifier: overriding a component library default
              is exactly what it is for. */}
          <Show when={props.allowDraft !== false}>
            <select
              class="select select-sm w-auto! border-base-300 bg-base-100"
              value={visibility()}
              disabled={busy()}
              aria-label="Visibility"
              onChange={(event) =>
                setVisibility(event.currentTarget.value as Visibility)
              }
            >
              <option value="public">Public</option>
              <option value="unlisted">Unlisted</option>
              <option value="draft">Draft</option>
            </select>
          </Show>

          {/* Feeds the familiar on youwin.dev and appears nowhere else. Left
              alone, the pet reads the text instead — which is why the empty
              option says "no mood" rather than defaulting to Neutral, a value
              that deliberately means something different. */}
          <select
            class="select select-sm w-auto! border-base-300 bg-base-100"
            value={mood() ?? ""}
            disabled={busy()}
            aria-label="Mood"
            title="Feeds the familiar. Not shown on the post."
            onChange={(event) =>
              setMood((event.currentTarget.value || null) as Mood | null)
            }
          >
            <option value="">Mood…</option>
            <For each={MOODS}>
              {(name) => <option value={name}>{MOOD_LABEL[name]}</option>}
            </For>
          </select>
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
