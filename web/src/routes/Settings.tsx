import { useNavigate } from "@solidjs/router";
import { createSignal, Show } from "solid-js";

import { api } from "../lib/api";
import { absolute } from "../lib/format";
import { clearApiCache, isStandalone } from "../lib/pwa";
import { clearSession, session } from "../lib/session";

export default function Settings() {
  const navigate = useNavigate();
  const [busy, setBusy] = createSignal(false);
  const [confirming, setConfirming] = createSignal(false);

  const signOut = async (everywhere: boolean) => {
    setBusy(true);
    try {
      if (everywhere) {
        await api.logoutAll();
      } else {
        await api.logout();
      }
      // Every route on this origin is authenticated, so a stale API cache is the
      // one way a signed-out device could still show posts.
      await clearApiCache();
      clearSession();
      navigate("/login", { replace: true });
    } finally {
      setBusy(false);
    }
  };

  return (
    <div class="flex flex-col gap-6">
      <h1 class="text-lg font-medium">Settings</h1>

      <Show when={session()}>
        {(me) => (
          <section class="rounded-box border border-base-300 bg-base-200 p-4 text-sm">
            <dl class="flex flex-col gap-2">
              <div class="flex justify-between gap-4">
                <dt class="text-secondary">This session began</dt>
                <dd>{absolute(me().session_started)}</dd>
              </div>
              <div class="flex justify-between gap-4">
                <dt class="text-secondary">Active sessions</dt>
                <dd>{me().active_sessions}</dd>
              </div>
              <div class="flex justify-between gap-4">
                <dt class="text-secondary">Running as</dt>
                <dd>{isStandalone() ? "Installed app" : "Browser tab"}</dd>
              </div>
            </dl>
          </section>
        )}
      </Show>

      <section class="flex flex-col gap-3">
        <button
          type="button"
          class="btn btn-sm w-fit"
          disabled={busy()}
          onClick={() => void signOut(false)}
        >
          Sign out
        </button>

        <Show
          when={confirming()}
          fallback={
            <button
              type="button"
              class="btn btn-ghost btn-sm w-fit text-error/70 hover:text-error"
              disabled={busy()}
              onClick={() => setConfirming(true)}
            >
              Sign out everywhere
            </button>
          }
        >
          <div class="flex items-center gap-2 text-sm">
            <span class="text-error">
              End every session, including this one?
            </span>
            <button
              type="button"
              class="btn btn-error btn-xs"
              onClick={() => void signOut(true)}
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
          </div>
        </Show>

        <p class="text-xs text-base-content/40">
          The password lives in the server's environment. To change it, run{" "}
          <code>youwin-server hash-password</code> and restart the service —
          existing sessions survive, so sign out everywhere afterwards if that is
          the point.
        </p>
      </section>
    </div>
  );
}
