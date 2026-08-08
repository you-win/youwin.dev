import { defineConfig } from "vite";
import solid from "vite-plugin-solid";
import tailwindcss from "@tailwindcss/vite";

// The authoring SPA at write.youwin.dev. Caddy serves this build off disk; axum
// never touches it. See DESIGN.md "Authoring frontend".
export default defineConfig({
  plugins: [solid(), tailwindcss()],

  build: {
    outDir: "dist/write",
    emptyOutDir: true,
  },

  server: {
    // Proxy to the authoring listener so the SPA and its API share an origin in
    // dev exactly as they do in prod. This is what keeps cookies behaving the
    // same in both, and why there is no CORS configuration anywhere.
    proxy: {
      "/api": "http://127.0.0.1:8081",
      "/preview": "http://127.0.0.1:8081",
      // /preview renders through the PUBLIC templates, so it links the public
      // site's stylesheet — which lives on the public listener, not this one.
      // Vite serves its own modules from /src and /@vite in dev and never uses
      // /assets, so forwarding it is free. Production solves this in the Caddy
      // write block; see deploy/Caddyfile.youwin.dev.
      "/assets": "http://127.0.0.1:8080",
    },
  },
});
