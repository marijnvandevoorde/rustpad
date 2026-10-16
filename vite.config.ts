import react from "@vitejs/plugin-react";
import { defineConfig } from "vite";
import wasm from "vite-plugin-wasm";

export default defineConfig({
  base: "",
  build: {
    // Top-level await (used to load the WebAssembly module) needs ES2022.
    target: "es2022",
    chunkSizeWarningLimit: 1000,
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
