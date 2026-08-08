/**
 * M0 placeholder. Its only job is to exercise every layer of the styling stack
 * at once, so a break is visible immediately rather than at M3:
 *
 *   - theme tokens on their own (base-100/200/300, base-content, secondary)
 *   - the shared `.post-body` component layer, which also styles server-rendered
 *     markup on the public site
 *   - a real DaisyUI component, which is the live check that DaisyUI picks up
 *     `--color-*` declared on `:root` by theme.css
 *
 * Routing, auth, and the composer arrive in M2/M3.
 */
export default function App() {
  return (
    <main class="mx-auto max-w-2xl px-4 py-12">
      <h1 class="text-2xl font-medium">write — youwin.dev</h1>
      <p class="mt-1 text-sm text-secondary">
        M0 skeleton. Composer lands in M3.
      </p>

      <div class="mt-8 rounded-box border border-base-300 bg-base-200 p-4">
        <div class="post-body">
          <p>
            Rendered post bodies look like this, with{" "}
            <a href="https://youwin.dev">a link</a>, some{" "}
            <code>inline code</code>, and enough text to show the measure.
          </p>
          <blockquote>Mist has no hard edges.</blockquote>
        </div>
      </div>

      {/* If this button is unstyled, DaisyUI is not seeing theme.css's :root
          custom properties — see the note in app.css for the fallback. */}
      <button type="button" class="btn btn-primary mt-6">
        DaisyUI button
      </button>
    </main>
  );
}
