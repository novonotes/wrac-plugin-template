import { defineConfig } from "vite";

export default defineConfig({
  server: {
    // Debug plugins load the WebView from 127.0.0.1. Vite's default `localhost`
    // may bind only to the IPv6 loopback in some environments, causing a mismatch
    // with the address the WebView inside the DAW resolves to, which can result in a black screen.
    host: "127.0.0.1",
    port: 5173,
    strictPort: true,
  },
  build: {
    outDir: "dist",
    emptyOutDir: true,
  },
});
