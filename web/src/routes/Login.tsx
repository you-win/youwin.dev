import { useNavigate } from "@solidjs/router";
import { createSignal, Show } from "solid-js";

import { api, ApiError } from "../lib/api";
import { flush } from "../lib/outbox";
import { loadSession } from "../lib/session";

export default function Login() {
  const navigate = useNavigate();
  const [password, setPassword] = createSignal("");
  const [busy, setBusy] = createSignal(false);
  const [error, setError] = createSignal<string | null>(null);

  const submit = async (event: Event) => {
    event.preventDefault();
    if (busy() || password().length === 0) return;

    setBusy(true);
    setError(null);
    try {
      await api.login(password());
      await loadSession();
      // Anything queued while the session was expired can go out now. Not
      // awaited — the feed is what you asked for, and the outbox shows its own
      // progress there.
      void flush();
      navigate("/", { replace: true });
    } catch (e) {
      // The server refuses a correct password while throttled, so say so
      // specifically — otherwise it reads as "I have forgotten my own password".
      if (e instanceof ApiError && e.code === "too_many_attempts") {
        setError("Too many attempts. Wait a while and try again.");
      } else {
        setError("That password is not right.");
      }
      setPassword("");
    } finally {
      setBusy(false);
    }
  };

  return (
    <div class="mx-auto max-w-sm py-16">
      <h1 class="text-xl font-medium">write</h1>
      <p class="mt-1 text-sm text-secondary">youwin.dev</p>

      <form class="mt-8 flex flex-col gap-3" onSubmit={submit}>
        <input
          type="password"
          class="input w-full border-base-300 bg-base-200"
          placeholder="Password"
          autocomplete="current-password"
          value={password()}
          disabled={busy()}
          // The only field on the page, and the page exists to be typed into.
          ref={(el) => queueMicrotask(() => el.focus())}
          onInput={(event) => setPassword(event.currentTarget.value)}
        />

        <Show when={error()}>
          {(message) => (
            <p class="text-sm text-error" role="alert">
              {message()}
            </p>
          )}
        </Show>

        <button
          type="submit"
          class="btn btn-primary"
          disabled={busy() || password().length === 0}
        >
          {busy() ? "…" : "Sign in"}
        </button>
      </form>
    </div>
  );
}
