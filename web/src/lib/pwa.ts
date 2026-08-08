/**
 * Service worker registration, update handling, and install prompting.
 *
 * Kept out of components so the shell only has to read two signals and call two
 * functions.
 */

import { createSignal } from "solid-js";
import { registerSW } from "virtual:pwa-register";

/** A new build is waiting. The user chooses when to take it. */
const [updateReady, setUpdateReady] = createSignal(false);

/** The browser offered an install prompt and we stashed it for a button. */
const [installable, setInstallable] = createSignal(false);

export { installable, updateReady };

let applyUpdate: ((reload?: boolean) => Promise<void>) | null = null;
let deferredInstall: BeforeInstallPromptEvent | null = null;

/**
 * Chrome's install prompt event, which TypeScript's DOM lib does not declare
 * because it is not in any spec.
 */
interface BeforeInstallPromptEvent extends Event {
  prompt(): Promise<void>;
  readonly userChoice: Promise<{ outcome: "accepted" | "dismissed" }>;
}

export function initPwa() {
  applyUpdate = registerSW({
    // `immediate` is load-bearing, not a tweak. Without it registerSW waits for
    // the window `load` event — but this runs from Solid's onMount, which in a
    // production build can fire after load has already passed, so the listener
    // never runs and the worker never registers. Dev hides this, because the
    // plugin injects its own registration there. The symptom is a PWA that
    // installs but has no offline support and never offers an update.
    immediate: true,

    // The worker is registered with skipWaiting off, so a new build sits in
    // waiting until this is called. Nothing reloads mid-sentence.
    onNeedRefresh: () => setUpdateReady(true),
  });

  window.addEventListener("beforeinstallprompt", (event) => {
    // Suppress the browser's own banner so the offer appears where it makes
    // sense, rather than over the composer.
    event.preventDefault();
    deferredInstall = event as BeforeInstallPromptEvent;
    setInstallable(true);
  });

  window.addEventListener("appinstalled", () => {
    deferredInstall = null;
    setInstallable(false);
  });
}

/** Activates the waiting worker and reloads. */
export async function update() {
  setUpdateReady(false);
  await applyUpdate?.(true);
}

export async function install() {
  if (!deferredInstall) return;
  await deferredInstall.prompt();
  await deferredInstall.userChoice;
  deferredInstall = null;
  setInstallable(false);
}

/**
 * Drops cached API responses.
 *
 * Called on sign-out: every route on this origin is authenticated, so a stale
 * cache is the one way a signed-out device could still show posts. Named for
 * the cache in vite.config.ts's runtimeCaching — they must stay in step.
 */
export async function clearApiCache() {
  if (!("caches" in window)) return;
  try {
    await caches.delete("youwin-api");
  } catch {
    // A failure here must not block signing out.
  }
}

/** True when running as an installed app rather than in a browser tab. */
export function isStandalone() {
  return window.matchMedia("(display-mode: standalone)").matches;
}
