import { defineConfig } from "vite";
import tailwindcss from "@tailwindcss/vite";

// The public archive's single stylesheet. No Solid plugin and no HTML entry —
// youwin.dev ships zero JavaScript, and its markup comes from maud on the server.
//
// The output is content-hashed and the manifest is emitted so the server can
// resolve the hashed URL at startup. That indirection is what keeps `cargo build`
// independent of whether pnpm has run.
export default defineConfig({
  plugins: [tailwindcss()],

  build: {
    outDir: "dist/public",
    emptyOutDir: true,
    manifest: true,
    rollupOptions: {
      input: "src/public.css",
    },
  },
});
