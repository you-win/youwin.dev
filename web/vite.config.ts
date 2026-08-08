import { defineConfig } from "vite";
import solid from "vite-plugin-solid";
import tailwindcss from "@tailwindcss/vite";
import { VitePWA } from "vite-plugin-pwa";

/** Approximates base-100, oklch(17% 0.016 162). Also in index.html and theme.css. */
const BASE_100 = "#09120d";

const AUTHORING = "http://127.0.0.1:8081";
const PUBLIC = "http://127.0.0.1:8080";

// The authoring SPA at write.youwin.dev. Caddy serves this build off disk; axum
// never touches it. See DESIGN.md "Authoring frontend".
export default defineConfig({
  plugins: [
    solid(),
    tailwindcss(),
    VitePWA({
      // "prompt", not "autoUpdate". An automatic reload can land mid-sentence
      // and take an unposted draft with it — the one loss this app must not
      // risk. The shell offers the update instead; see app.tsx.
      registerType: "prompt",
      filename: "sw.js",
      includeAssets: ["icons/apple-touch-icon.png"],

      manifest: {
        name: "write — youwin.dev",
        short_name: "write",
        description: "Write posts for youwin.dev.",
        start_url: "/",
        scope: "/",
        display: "standalone",
        orientation: "portrait",
        background_color: BASE_100,
        theme_color: BASE_100,
        icons: [
          { src: "/icons/icon-192.png", sizes: "192x192", type: "image/png" },
          { src: "/icons/icon-512.png", sizes: "512x512", type: "image/png" },
          {
            src: "/icons/icon-maskable-512.png",
            sizes: "512x512",
            type: "image/png",
            // Launchers crop this to a circle; the artwork is inset to survive it.
            purpose: "maskable",
          },
        ],
      },

      workbox: {
        // Without the denylist the service worker answers API calls with the
        // HTML shell and every fetch dies at JSON.parse. /preview is denied for
        // the same reason from the other direction: it is server-rendered HTML
        // living on this origin, not an SPA route, so the shell must not stand
        // in for it.
        navigateFallback: "/index.html",
        navigateFallbackDenylist: [/^\/api\//, /^\/preview\//],

        // The new worker waits rather than seizing control, so an open composer
        // is never swapped out from under itself.
        skipWaiting: false,
        clientsClaim: false,

        runtimeCaching: [
          {
            // Only the post endpoints, and only GET — `method` below means a
            // POST/PATCH/DELETE never reaches the worker at all. /api/auth/* is
            // deliberately absent: caching it would let a logged-out phone keep
            // answering as though it were signed in.
            urlPattern: ({ url, sameOrigin }) =>
              sameOrigin && url.pathname.startsWith("/api/posts"),
            method: "GET",
            handler: "NetworkFirst",
            options: {
              cacheName: "youwin-api",
              // Fall back to cache rather than hanging on a dead connection.
              networkTimeoutSeconds: 5,
              expiration: { maxEntries: 64, maxAgeSeconds: 60 * 60 * 24 * 7 },
              cacheableResponse: { statuses: [200] },
            },
          },
        ],
      },

      // Registers a worker in dev so the update and install paths are
      // exercisable. Note the dev worker is a stripped one: it honours
      // navigateFallback but NOT runtimeCaching, so the API cache simply does
      // not exist here. Verify offline behaviour against `pnpm run preview`,
      // which serves the real build — concluding "caching is broken" from a dev
      // session would be wrong.
      devOptions: { enabled: true, type: "module", navigateFallback: "/index.html" },
    }),
  ],

  build: {
    outDir: "dist/write",
    emptyOutDir: true,
  },

  // Proxy to the authoring listener so the SPA and its API share an origin in
  // dev exactly as they do in prod. That is what keeps cookies behaving the
  // same in both, and why there is no CORS configuration anywhere.
  server: {
    proxy: {
      "/api": AUTHORING,
      "/preview": AUTHORING,
      // /preview renders through the PUBLIC templates, so it links the public
      // stylesheet, which lives on the other listener. In dev Vite serves its
      // own modules from /src and /@vite and never touches /assets, so claiming
      // the whole prefix here is free.
      "/assets": PUBLIC,
    },
  },

  // `vite preview` serves the real build, service worker and all — offline
  // behaviour has to be verified against the worker that actually ships, since
  // the dev worker omits runtimeCaching.
  //
  // The proxy is deliberately NOT the same as the dev one. Here the SPA's own
  // bundle lives under /assets, so forwarding that prefix wholesale sends the
  // app's JavaScript to the public listener and nothing mounts at all. Only the
  // public stylesheet is forwarded, matched by its filename prefix — which is
  // exactly what the Caddy write block does with `handle /assets/public-*`.
  preview: {
    port: 4173,
    proxy: {
      "/api": AUTHORING,
      "/preview": AUTHORING,
      "/assets/public-": PUBLIC,
    },
  },
});
