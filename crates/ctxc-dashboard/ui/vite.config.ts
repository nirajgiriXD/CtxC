import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";

// The build output is embedded in the `ctxc` binary by build.rs, so everything
// has to be relative: the dashboard is served from the daemon's root today, but
// nothing should break if it ever moves under a prefix.
export default defineConfig({
  plugins: [react(), tailwindcss()],
  base: "./",
  build: {
    outDir: "dist",
    emptyOutDir: true,
    // One JS file and one CSS file keep the embedded asset table small and the
    // first paint quick. There is no route-splitting to gain from: the whole
    // dashboard is a few screens.
    rollupOptions: {
      output: {
        manualChunks: undefined,
      },
    },
  },
  server: {
    // `npm run dev` proxies to a daemon started separately, so the dashboard
    // can be worked on without rebuilding the Rust binary each time.
    proxy: {
      "/v1": {
        target: "http://127.0.0.1:7717",
        changeOrigin: true,
        ws: true,
      },
    },
  },
});
