/**
 * The typed edge of the authoring API.
 *
 * Same origin as the SPA in dev (via Vite's proxy) and in production, so cookies
 * travel without any CORS or `credentials` gymnastics.
 *
 * Every 401 is intercepted here and handed to a single registered callback, so
 * no component ever has to think about an expired session.
 */

export type Visibility = "public" | "unlisted" | "draft";

/**
 * How a post was written, picked in the composer.
 *
 * Never shown on youwin.dev — it feeds the familiar, which is the kaomoji on the
 * public feed. `null` means the picker was left alone, and the familiar infers a
 * mood from the text instead; an explicit "neutral" means "nothing to report"
 * and turns that inference off.
 */
export type Mood =
  | "content"
  | "contemplative"
  | "tired"
  | "excited"
  | "melancholy"
  | "chaos"
  | "neutral";

/** In the order the picker offers them, matching the server's `Mood::ALL`. */
export const MOODS: readonly Mood[] = [
  "content",
  "contemplative",
  "tired",
  "excited",
  "melancholy",
  "chaos",
  "neutral",
];

export interface Post {
  id: string;
  body: string;
  body_html: string;
  visibility: Visibility;
  mood: Mood | null;
  is_reply: boolean;
  reply_count: number;
  created_at: number;
  edited_at: number | null;
}

export interface Page {
  posts: Post[];
  next: string | null;
}

/**
 * A post inside a thread, with how far under the root it hangs.
 *
 * `depth` is computed server-side (crates/server/src/thread.rs) rather than
 * derived here from parent ids, so this view and youwin.dev cannot disagree
 * about the shape of a thread — including the case that would be easiest to get
 * differently wrong twice, a reply whose parent has been deleted.
 */
export interface ThreadItem extends Post {
  depth: number;
}

export interface Thread {
  post: Post;
  thread: ThreadItem[];
}

export interface Me {
  authenticated: boolean;
  session_started: number;
  active_sessions: number;
}

export class ApiError extends Error {
  constructor(
    readonly status: number,
    readonly code: string,
    message: string,
  ) {
    super(message);
    this.name = "ApiError";
  }
}

/**
 * The server could not be reached at all.
 *
 * Distinct from `ApiError` on purpose: "the server said no" and "there is no
 * server right now" call for opposite responses. Conflating them signs you out
 * every time you walk into a tunnel.
 */
export class NetworkError extends Error {
  constructor(message = "Could not reach the server.") {
    super(message);
    this.name = "NetworkError";
  }
}

let onUnauthorized: (() => void) | null = null;

/** Registered once by the app shell; see `app.tsx`. */
export function setUnauthorizedHandler(handler: () => void) {
  onUnauthorized = handler;
}

async function request<T>(
  method: string,
  path: string,
  body?: unknown,
): Promise<T> {
  let response: Response;
  try {
    response = await fetch(path, {
      method,
      headers: body === undefined ? {} : { "content-type": "application/json" },
      body: body === undefined ? undefined : JSON.stringify(body),
    });
  } catch {
    // fetch rejects only when the request never completed — offline, DNS
    // failure, connection refused. An HTTP error status resolves normally and
    // is handled below.
    throw new NetworkError();
  }

  if (response.status === 401) {
    // One place handles expiry. Components see a thrown error and can ignore it;
    // the shell has already started routing to /login.
    onUnauthorized?.();
    throw new ApiError(401, "unauthorized", "Session expired.");
  }

  if (response.status === 204) return undefined as T;

  let payload: unknown;
  try {
    payload = await response.json();
  } catch {
    payload = null;
  }

  if (!response.ok) {
    const error = (payload as { error?: { code?: string; message?: string } })
      ?.error;
    throw new ApiError(
      response.status,
      error?.code ?? "unknown",
      error?.message ?? `Request failed (${response.status}).`,
    );
  }

  return payload as T;
}

export const api = {
  me: () => request<Me>("GET", "/api/auth/me"),

  login: (password: string) =>
    request<{ authenticated: boolean }>("POST", "/api/auth/login", { password }),

  logout: () => request<unknown>("POST", "/api/auth/logout"),

  logoutAll: () =>
    request<{ sessions_ended: number }>("POST", "/api/auth/logout-all"),

  feed: (cursor?: string | null) =>
    request<Page>(
      "GET",
      cursor ? `/api/posts?cursor=${encodeURIComponent(cursor)}` : "/api/posts",
    ),

  drafts: () => request<Page>("GET", "/api/drafts"),

  /**
   * Searches everything, drafts included.
   *
   * The server takes whatever is typed and reduces it to quoted tokens, so no
   * input here can produce an error status — there is nothing for the caller to
   * validate before sending.
   */
  search: (query: string, cursor?: string | null) => {
    const params = new URLSearchParams({ q: query });
    if (cursor) params.set("cursor", cursor);
    return request<Page>("GET", `/api/search?${params}`);
  },

  show: (id: string) =>
    request<Thread>("GET", `/api/posts/${encodeURIComponent(id)}`),

  create: (
    body: string,
    visibility: Visibility,
    mood: Mood | null,
    parentId?: string,
  ) =>
    request<Post>("POST", "/api/posts", {
      body,
      visibility,
      mood,
      parent_id: parentId,
    }),

  /**
   * Omitting a key leaves that field alone. `mood: null` is therefore not the
   * same as omitting it — that one clears the mood back to "did not say", which
   * JSON.stringify preserves because an explicit null is serialized and an
   * `undefined` is dropped.
   */
  update: (
    id: string,
    changes: { body?: string; visibility?: Visibility; mood?: Mood | null },
  ) =>
    request<Post>("PATCH", `/api/posts/${encodeURIComponent(id)}`, changes),

  destroy: (id: string) =>
    request<{ deleted: number }>(
      "DELETE",
      `/api/posts/${encodeURIComponent(id)}`,
    ),
};

/** Where a post lives, or will live, on the public site. */
export function previewUrl(id: string) {
  return `/preview/${encodeURIComponent(id)}`;
}

/**
 * The deployed public site.
 *
 * Hardcoded rather than derived from `location`: a shared link must point at
 * youwin.dev whether it was composed from the deployed app or from localhost.
 */
export const PUBLIC_ORIGIN = "https://youwin.dev";

export function publicUrl(id: string) {
  return `${PUBLIC_ORIGIN}/p/${encodeURIComponent(id)}`;
}

/**
 * Shares a post's public URL, falling back to the clipboard.
 *
 * Resolves to what actually happened so the caller can confirm a copy — a share
 * sheet is self-evident, a silent clipboard write is not.
 */
export async function share(id: string): Promise<"shared" | "copied" | "failed"> {
  const url = publicUrl(id);

  if (navigator.share) {
    try {
      await navigator.share({ url });
      return "shared";
    } catch (e) {
      // Dismissing the sheet throws AbortError; that is a choice, not a failure,
      // so it must not fall through to a surprise clipboard write.
      if (e instanceof DOMException && e.name === "AbortError") return "failed";
    }
  }

  try {
    await navigator.clipboard.writeText(url);
    return "copied";
  } catch {
    return "failed";
  }
}
