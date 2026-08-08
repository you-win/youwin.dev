/**
 * Auth state, as one module-level signal.
 *
 * A context would be the reflex here, but there is exactly one session and no
 * SSR, so a shared signal is the whole feature — and it can be read from
 * `api.ts`'s 401 interceptor without threading a provider through it.
 *
 * `undefined` means "not yet known", which is distinct from `null` ("known to be
 * logged out"). Conflating them would flash the login form on every reload.
 */

import { createSignal } from "solid-js";

import { api, type Me } from "./api";

const [session, setSession] = createSignal<Me | null | undefined>(undefined);

export { session };

/** Fetches the session. Never throws — a failure is simply "logged out". */
export async function loadSession(): Promise<Me | null> {
  try {
    const me = await api.me();
    setSession(me);
    return me;
  } catch {
    setSession(null);
    return null;
  }
}

export function clearSession() {
  setSession(null);
}
