/**
 * Posts written with no connection, and the retry that eventually sends them.
 *
 * The offline feed reads from the service worker's cache, which has always
 * worked. Writing did not: the composer surfaced an error and kept your text in
 * the box, which is honest but means a post written on the underground exists
 * only for as long as you leave that tab alone.
 *
 * So a post that cannot be sent is queued instead, in `localStorage`, and
 * flushed when the connection comes back.
 *
 * **Retry only on a network failure.** A flush cannot tell "the request never
 * arrived" from "the reply never came back", which is what the idempotency key
 * is for — the server returns the post the first attempt wrote rather than
 * writing a second one. But an *answered* request is a decision, and repeating
 * it forever would turn one bad post into a permanent loop. Anything the server
 * replied to leaves the queue: a 422 for an over-long body, a 401 that the API
 * wrapper has already routed to the login page, a 404 for a queued post that was
 * published and then deleted from another device.
 *
 * **Nothing is ever dropped silently.** A rejected post keeps its text and moves
 * to `rejected()`, where the feed shows it with the reason and a way to put it
 * back in the composer. Losing writing is the one failure this application
 * cannot have.
 */

import { createSignal } from "solid-js";

import { ApiError, api, NetworkError, type Mood, type Visibility } from "./api";

const STORAGE_KEY = "youwin.outbox";

/**
 * Statuses that mean "not now" rather than "no".
 *
 * A reverse proxy answers these when the origin is down, restarting, or
 * overloaded — the request never reached the application, so nothing about it
 * was refused. Deliberately not 500: that one *did* reach the application, and
 * repeating a request that crashes it is how one bad post becomes a loop.
 */
const RETRYABLE = new Set([502, 503, 504]);

export interface Queued {
  /** The idempotency key, generated once and reused on every attempt. */
  key: string;
  body: string;
  visibility: Visibility;
  mood: Mood | null;
  /** Public id of the post this answers, for a queued reply. */
  parentId?: string;
  queuedAt: number;
  /** Why the server refused it. Present only on `rejected()` entries. */
  error?: string;
}

const [queued, setQueued] = createSignal<Queued[]>([]);
const [rejected, setRejected] = createSignal<Queued[]>([]);

export { queued, rejected };

/** Called with each post that makes it out, so the feed can show the real one. */
type Listener = (post: Awaited<ReturnType<typeof api.create>>) => void;
const listeners = new Set<Listener>();

export function onFlushed(listener: Listener) {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

interface Stored {
  queued: Queued[];
  rejected: Queued[];
}

/**
 * Reads the queue back at startup.
 *
 * Anything malformed is discarded rather than thrown: a storage entry from an
 * older build must not be able to stop the app from loading, which is the one
 * outcome worse than losing the queue it describes.
 */
export function loadOutbox() {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (!raw) return;

    const parsed = JSON.parse(raw) as Partial<Stored>;
    setQueued(Array.isArray(parsed.queued) ? parsed.queued : []);
    setRejected(Array.isArray(parsed.rejected) ? parsed.rejected : []);
  } catch {
    setQueued([]);
    setRejected([]);
  }
}

function persist() {
  try {
    const state: Stored = { queued: queued(), rejected: rejected() };
    localStorage.setItem(STORAGE_KEY, JSON.stringify(state));
  } catch {
    // Private browsing, or a full quota. The queue still works for this
    // session; it just will not survive a reload, which is strictly better than
    // refusing the post.
  }
}

/**
 * A key the server will accept: 36 characters, unguessable, generated once.
 *
 * `randomUUID` needs a secure context, which `https://write.youwin.dev` and
 * `http://localhost` both are. The fallback exists for anything else — it is not
 * cryptographically strong, and does not need to be. Two colliding keys would
 * mean one post silently replacing another, so the fallback still takes its
 * entropy from `getRandomValues` where that exists.
 */
function newKey(): string {
  if (typeof crypto?.randomUUID === "function") return crypto.randomUUID();

  const bytes = new Uint8Array(16);
  crypto?.getRandomValues?.(bytes);
  return Array.from(bytes, (b) => b.toString(16).padStart(2, "0")).join("");
}

/** Adds a post to the queue and returns it, so the caller can show it at once. */
export function enqueue(
  body: string,
  visibility: Visibility,
  mood: Mood | null,
  parentId?: string,
): Queued {
  const item: Queued = {
    key: newKey(),
    body,
    visibility,
    mood,
    parentId,
    queuedAt: Date.now(),
  };

  setQueued((current) => [...current, item]);
  persist();
  return item;
}

/** Puts a rejected post back where it can be edited, and forgets it here. */
export function discard(key: string) {
  setRejected((current) => current.filter((item) => item.key !== key));
  persist();
}

let flushing = false;

/**
 * Sends everything queued, oldest first.
 *
 * Serial rather than concurrent: the queue is ordered, and a queued reply must
 * not be able to arrive before the post it answers. It also means a connection
 * that dies halfway leaves the rest of the queue untouched rather than firing a
 * burst of requests into a closing tunnel.
 *
 * Safe to call at any time — a second call while one is in flight returns
 * immediately rather than sending anything twice.
 */
export async function flush(): Promise<void> {
  if (flushing) return;
  flushing = true;

  try {
    for (;;) {
      const item = queued()[0];
      if (!item) return;

      try {
        const post = await api.create(
          item.body,
          item.visibility,
          item.mood,
          item.parentId,
          item.key,
        );

        setQueued((current) => current.filter((q) => q.key !== item.key));
        persist();
        for (const listener of listeners) listener(post);
      } catch (error) {
        if (error instanceof NetworkError) {
          // Still offline. Everything stays queued, in order, and the next
          // `online` event tries again.
          return;
        }

        if (error instanceof ApiError && error.status === 401) {
          // The session expired while this sat in the queue. That is not a
          // rejection — the post was never seen — so it stays queued and goes
          // out after signing back in. The API wrapper has already started
          // routing to /login, and `flush` is called again from there.
          return;
        }

        if (error instanceof ApiError && RETRYABLE.has(error.status)) {
          // Caddy answering for an origin that is down or restarting. The
          // device has a connection, so `fetch` resolves rather than rejecting
          // — which means "unreachable" does not always arrive as a
          // NetworkError, and treating a 502 as a refusal would reject a
          // perfectly good post because a deploy was mid-restart.
          return;
        }

        // The server answered, so this will not get better by repeating it.
        const message =
          error instanceof ApiError
            ? error.message
            : "Could not post, and the reason is unclear.";

        setQueued((current) => current.filter((q) => q.key !== item.key));
        setRejected((current) => [...current, { ...item, error: message }]);
        persist();
      }
    }
  } finally {
    flushing = false;
  }
}

/**
 * Wires the queue to the browser's own idea of being online.
 *
 * `online` fires on regaining a network, and the load-time flush covers the
 * case that actually happens most: the app was closed offline and reopened
 * somewhere with signal, where no event ever fires.
 *
 * Background Sync would let the worker do this with the app closed, but
 * `generateSW` cannot register a sync handler without switching to
 * `injectManifest` and hand-writing the worker. For a queue that exists to
 * survive a train tunnel, "sends when you next open it" is the same outcome.
 */
export function initOutbox() {
  loadOutbox();
  window.addEventListener("online", () => void flush());
  if (navigator.onLine) void flush();
}
