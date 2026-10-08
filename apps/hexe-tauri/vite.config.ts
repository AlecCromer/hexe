import { defineConfig } from "vite";

// @ts-expect-error process is a nodejs global
const host = process.env.TAURI_DEV_HOST;

// Tauri dev server: fixed port (tauri.conf.json devUrl), UI root is ./ui, output ./dist (frontendDist).
export default defineConfig({
  root: "ui",
  clearScreen: false,
  build: { outDir: "../dist", emptyOutDir: true, target: "safari15" },
  server: {
    port: 1520,
    strictPort: true,
    host: host || false,
    hmr: host ? { protocol: "ws", host, port: 1521 } : undefined,
    watch: { ignored: ["**/src-tauri/**"] },
  },
});
