import {
  A,
  Navigate,
  Route,
  Router,
  useLocation,
  useNavigate,
  type RouteSectionProps,
} from "@solidjs/router";
import { createSignal, onCleanup, onMount, Show } from "solid-js";

import { setUnauthorizedHandler } from "./lib/api";
import { initPwa, install, installable, update, updateReady } from "./lib/pwa";
import { clearSession, loadSession, session } from "./lib/session";
import Drafts from "./routes/Drafts";
import Feed from "./routes/Feed";
import Login from "./routes/Login";
import Permalink from "./routes/Permalink";
import Search from "./routes/Search";
import Settings from "./routes/Settings";

/**
 * The shell: navigation, and the one place that decides whether you are allowed
 * to see anything.
 *
 * `session()` is `undefined` while unknown, `null` once known to be logged out.
 * Distinguishing those is what stops the login form flashing on every reload.
 */
function Shell(props: RouteSectionProps) {
  const location = useLocation();
  const navigate = useNavigate();
  const [offline, setOffline] = createSignal(!navigator.onLine);

  onMount(() => {
    void loadSession();
    initPwa();

    // Any 401, from any request, lands here — so no component has to handle an
    // expired session, and none of them can forget to.
    setUnauthorizedHandler(() => {
      clearSession();
      if (location.pathname !== "/login") {
        navigate("/login", { replace: true });
      }
    });

    const online = () => setOffline(false);
    const dropped = () => setOffline(true);
    window.addEventListener("online", online);
    window.addEventListener("offline", dropped);
    onCleanup(() => {
      window.removeEventListener("online", online);
      window.removeEventListener("offline", dropped);
    });
  });

  const onLoginPage = () => location.pathname === "/login";

  return (
    <div class="mx-auto flex min-h-dvh max-w-2xl flex-col px-4">
      <Show when={!onLoginPage()}>
        <header class="flex items-baseline justify-between border-b border-base-300 py-6">
          <A href="/" class="text-lg font-medium no-underline">
            write
          </A>
          <nav class="flex items-center gap-4 text-sm text-secondary">
            <Show when={installable()}>
              <button
                type="button"
                class="btn btn-ghost btn-xs text-primary"
                onClick={() => void install()}
              >
                install
              </button>
            </Show>
            <A href="/search">search</A>
            <A href="/drafts">drafts</A>
            <A href="/settings">settings</A>
            <a
              href="https://youwin.dev"
              target="_blank"
              rel="noopener"
              title="The public site"
            >
              live ↗
            </a>
          </nav>
        </header>
      </Show>

      {/* Offline is worth saying plainly: the feed still reads from cache, but
          posting will fail, and finding that out by losing a draft is not the
          way to learn it. */}
      <Show when={offline()}>
        <div class="mt-4 rounded-box border border-warning/40 bg-warning/10 px-4 py-2 text-sm text-warning">
          Offline. You can read, but posting will not work until you reconnect.
        </div>
      </Show>

      <Show when={updateReady()}>
        <div class="mt-4 flex items-center justify-between gap-3 rounded-box border border-primary/40 bg-base-200 px-4 py-2 text-sm">
          <span>A new version is ready.</span>
          <button
            type="button"
            class="btn btn-primary btn-xs"
            onClick={() => void update()}
          >
            Reload
          </button>
        </div>
      </Show>

      <main class="flex-1 py-8">
        <Show
          when={session() !== undefined}
          fallback={
            <p class="py-16 text-center text-sm text-secondary">Loading…</p>
          }
        >
          <Show
            when={session() !== null || onLoginPage()}
            fallback={<Navigate href="/login" />}
          >
            {props.children}
          </Show>
        </Show>
      </main>
    </div>
  );
}

export default function App() {
  return (
    <Router root={Shell}>
      <Route path="/" component={Feed} />
      <Route path="/p/:id" component={Permalink} />
      <Route path="/search" component={Search} />
      <Route path="/drafts" component={Drafts} />
      <Route path="/settings" component={Settings} />
      <Route path="/login" component={Login} />
      <Route
        path="*"
        component={() => (
          <p class="py-16 text-center text-sm text-secondary">Not found.</p>
        )}
      />
    </Router>
  );
}
