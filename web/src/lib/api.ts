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

export interface Post {
  id: string;
  body: string;
  body_html: string;
  visibility: Visibility;
  is_reply: boolean;
  reply_count: number;
  created_at: number;
  edited_at: number | null;
}

export interface Page {
  posts: Post[];
  next: string | null;
}

export interface Thread {
  post: Post;
  thread: Post[];
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
  const response = await fetch(path, {
    method,
    headers: body === undefined ? {} : { "content-type": "application/json" },
    body: body === undefined ? undefined : JSON.stringify(body),
  });

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

  show: (id: string) =>
    request<Thread>("GET", `/api/posts/${encodeURIComponent(id)}`),

  create: (body: string, visibility: Visibility, parentId?: string) =>
    request<Post>("POST", "/api/posts", {
      body,
      visibility,
      parent_id: parentId,
    }),

  update: (
    id: string,
    changes: { body?: string; visibility?: Visibility },
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
