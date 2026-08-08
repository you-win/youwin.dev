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

import { api, NetworkError, type Me } from "./api";

/**
 * The last session the server confirmed.
 *
 * Kept so that being offline does not read as being signed out. `/api/auth/me`
 * is deliberately never cached by the service worker — caching it would let a
 * signed-out device keep answering as though it were signed in — so offline
 * there is nothing to ask. Without this the app bounces to a login form that
 * cannot even be submitted, while a perfectly readable cached feed sits behind
 * it.
 */
const LAST_KNOWN = "youwin:last-session";

const [session, setSession] = createSignal<Me | null | undefined>(undefined);

export { session };

function remember(me: Me) {
  try {
    localStorage.setItem(LAST_KNOWN, JSON.stringify(me));
  } catch {
    // Private mode, quota, disabled storage — none of it should break login.
  }
}

function recall(): Me | null {
  try {
    const stored = localStorage.getItem(LAST_KNOWN);
    return stored ? (JSON.parse(stored) as Me) : null;
  } catch {
    return null;
  }
}

/**
 * Fetches the session. Never throws.
 *
 * A 401 means signed out. A network failure means unknown — and falls back to
 * the last confirmed session, which is safe because the only way to have one is
 * to have signed in on this device, and signing out clears it along with the
 * cached API responses.
 */
export async function loadSession(): Promise<Me | null> {
  try {
    const me = await api.me();
    setSession(me);
    remember(me);
    return me;
  } catch (error) {
    if (error instanceof NetworkError) {
      const remembered = recall();
      if (remembered) {
        setSession(remembered);
        return remembered;
      }
    }

    clearSession();
    return null;
  }
}

export function clearSession() {
  setSession(null);
  try {
    localStorage.removeItem(LAST_KNOWN);
  } catch {
    // Nothing to do; the signal above is what gates the UI.
  }
}
