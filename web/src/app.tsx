import {
  A,
  Navigate,
  Route,
  Router,
  useLocation,
  useNavigate,
  type RouteSectionProps,
} from "@solidjs/router";
import { onMount, Show } from "solid-js";

import { setUnauthorizedHandler } from "./lib/api";
import { clearSession, loadSession, session } from "./lib/session";
import Drafts from "./routes/Drafts";
import Feed from "./routes/Feed";
import Login from "./routes/Login";
import Permalink from "./routes/Permalink";
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

  onMount(() => {
    void loadSession();

    // Any 401, from any request, lands here — so no component has to handle an
    // expired session, and none of them can forget to.
    setUnauthorizedHandler(() => {
      clearSession();
      if (location.pathname !== "/login") {
        navigate("/login", { replace: true });
      }
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
