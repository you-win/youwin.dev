import { createEffect, createSignal, on, onCleanup, Show } from "solid-js";

import { api, type FamiliarState, type Mood, type Visibility } from "../lib/api";

/**
 * The pet, beside the composer, reacting to what you are typing.
 *
 * It has always lived on youwin.dev, which is where the archive is read and not
 * where it is written — so as a reason to keep posting it closed its loop on a
 * surface you visit occasionally. Here it answers the question the public site
 * cannot ask: what the post you have not made yet would do to it.
 *
 * Two readings are held rather than one. `resting` is the pet as it is;
 * `prospective` is the pet the current draft would produce, and the difference
 * between them is the only interesting thing on screen. Recomputing a single
 * value would leave nothing to compare against and turn the whole component into
 * a slower copy of the public widget.
 */

/**
 * Quiet before asking the server what a draft would do.
 *
 * Long enough that a sentence is one request rather than forty, short enough
 * that the pet still feels like it is watching. The reply is a read of a table
 * this site keeps in page cache, on a host with exactly one user, so the cost of
 * being wrong here is small in both directions.
 */
const SETTLE_MS = 400;

export interface Draft {
  body: string;
  visibility: Visibility;
  mood: Mood | null;
}

interface Props {
  /** The composer's current contents. */
  draft: Draft | null;
  /**
   * Bumped by the parent after any write, to refetch the resting pet.
   *
   * A number rather than a callback ref because the parent already knows when it
   * has changed the archive and this component never does — it has no way to
   * observe a post being made from inside itself.
   */
  revision: number;
}

export default function Familiar(props: Props) {
  const [resting, setResting] = createSignal<FamiliarState | null>(null);
  const [prospective, setProspective] = createSignal<FamiliarState | null>(null);

  /** What is drawn: the draft's pet when there is one, otherwise the real one. */
  const shown = () => prospective() ?? resting();
  const previewing = () => prospective() !== null;

  const loadResting = async () => {
    try {
      setResting(await api.familiar());
    } catch {
      // Silent. The pet is decoration on this surface — a failed fetch must not
      // put an error banner above a composer that works perfectly well.
      setResting(null);
    }
  };

  createEffect(
    on(
      () => props.revision,
      () => void loadResting(),
    ),
  );

  createEffect(
    on(
      () => props.draft,
      (draft) => {
        if (!draft || draft.body.trim() === "") {
          setProspective(null);
          return;
        }

        const timer = setTimeout(() => {
          api
            .familiarDraft(draft.body, draft.visibility, draft.mood)
            .then(setProspective)
            .catch(() => setProspective(null));
        }, SETTLE_MS);

        // Every keystroke replaces the pending request rather than queueing one.
        onCleanup(() => clearTimeout(timer));
      },
    ),
  );

  /**
   * What posting this would visibly change, in the order it would be noticed.
   *
   * Only the dimensions that show on the face or the silhouette. Energy moves on
   * every post by construction, so reporting it would mean this line never said
   * anything specific.
   */
  const changes = () => {
    const before = resting();
    const after = prospective();
    if (!before || !after) return [];

    const moved: string[] = [];
    if (after.stage !== before.stage) moved.push(`grows to ${after.stage}`);
    if (after.form !== before.form) moved.push(`turns ${after.form}`);
    if (after.mood !== before.mood) moved.push(after.mood);
    if (after.level !== before.level) moved.push(after.level);
    return moved;
  };

  /** The one line under the pet while a draft is in the box. */
  const note = () => {
    const draft = props.draft;
    if (draft && draft.visibility !== "public") {
      return `${draft.visibility === "draft" ? "Drafts" : "Unlisted posts"} don't feed it.`;
    }

    const moved = changes();
    if (moved.length > 0) return `Posting this: ${moved.join(" · ")}.`;

    const growth = prospective()?.growth;
    if (growth) return `Posting this: ${growth.percent}% toward ${growth.toward}.`;
    return "Posting this: one more post.";
  };

  return (
    <Show when={shown()}>
      {(pet) => (
        <div
          class="flex items-center gap-4 rounded-box border p-4 transition-colors"
          classList={{
            "border-base-300 bg-base-200": !previewing(),
            "border-primary/50 bg-base-200": previewing(),
          }}
        >
          {/* Hidden from assistive technology: the text beside it says
              everything it says, and a screen reader spelling out `( ◕ ω ◕ )`
              character by character is noise. Same choice the public feed's
              widget makes. */}
          <pre
            aria-hidden="true"
            class="shrink-0 text-center font-mono text-sm leading-snug text-primary"
          >
            {pet().lines.join("\n")}
          </pre>

          <div class="min-w-0 text-sm">
            <Show
              when={pet().posts > 0}
              fallback={<p class="text-secondary">Waiting for a first post.</p>}
            >
              <p>
                {pet().mood} · {pet().level}
              </p>
              <p class="text-secondary">
                {pet().stage} {pet().form} · {pet().phase} hours
              </p>
            </Show>

            <p
              class="text-secondary"
              classList={{ "text-primary": previewing() }}
            >
              <Show
                when={previewing()}
                fallback={
                  // Speech displaces the counts rather than adding a line, the
                  // same way it does on the public feed — it only appears when
                  // something is genuinely unusual, and the two surfaces saying
                  // it differently would be two rules to keep true.
                  <Show
                    when={pet().speech}
                    fallback={
                      <>
                        {pet().posts} posts
                        <Show when={pet().streak_alive && pet().streak_days > 1}>
                          {" · "}
                          {pet().streak_days} day streak 🔥
                        </Show>
                      </>
                    }
                  >
                    {(said) => <>{said()}</>}
                  </Show>
                }
              >
                {note()}
              </Show>
            </p>
          </div>
        </div>
      )}
    </Show>
  );
}
