import react from "@vitejs/plugin-react";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { defineConfig } from "vite";
import wasm from "vite-plugin-wasm";

// The editor loads Monaco from a CDN; use the version in package-lock.json, so
// updating the monaco-editor package updates the editor.
const monacoVersion: string = JSON.parse(
  readFileSync(
    new URL("node_modules/monaco-editor/package.json", import.meta.url),
    "utf8",
  ),
).version;

export default defineConfig({
  base: "",
  define: {
    __MONACO_VERSION__: JSON.stringify(monacoVersion),
  },
  build: {
    // Top-level await (used to load the WebAssembly module) needs ES2022.
    target: "es2022",
    chunkSizeWarningLimit: 1000,
    rollupOptions: {
      input: {
        main: fileURLToPath(new URL("index.html", import.meta.url)),
        documents: fileURLToPath(new URL("documents.html", import.meta.url)),
      },
    },
  },
  plugins: [wasm(), react()],
  server: {
    proxy: {
      "/api": {
        target: "http://127.0.0.1:3030",
        changeOrigin: true,
        secure: false,
        ws: true,
      },
    },
  },
});
