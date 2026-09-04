import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwind from "@tailwindcss/vite";
import { fileURLToPath, URL } from "node:url";

// Tauri serves the built assets from disk; there is no dev server in a shipped
// build and no network access at any point (SPEC 4).
export default defineConfig({
  plugins: [react(), tailwind()],
  resolve: {
    alias: { "@": fileURLToPath(new URL("./src", import.meta.url)) },
  },
  clearScreen: false,
  server: {
    port: 5173,
    strictPort: true,
    watch: {
      // Vite watches the whole project root by default, and `target/` is
      // where `cargo` writes and constantly rewrites build output — including
      // the `.exe` Tauri's dev command is actively linking. Windows locks a
      // file while it is being written (Unix does not), so an unfiltered
      // watcher intermittently throws EBUSY there and kills `tauri dev`. None
      // of `target/` is frontend source, so it never needed watching.
      ignored: ["**/target/**", "**/src-tauri/gen/**"],
    },
  },
  build: {
    target: "chrome110",
    // Debug builds keep sourcemaps; a shipped build does not ship its sources.
    sourcemap: process.env["TAURI_ENV_DEBUG"] === "true",
    // Vite 8 minifies with oxc; naming esbuild would pull in a second toolchain.
    minify: process.env["TAURI_ENV_DEBUG"] !== "true",
    outDir: "dist",
    emptyOutDir: true,
  },
});
