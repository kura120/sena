import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import { resolve } from "path";

const host = process.env.TAURI_DEV_HOST;

export default defineConfig(async () => ({
  plugins: [react()],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    hmr: host
      ? {
          protocol: "ws",
          host,
          port: 1421,
        }
      : undefined,
    watch: {
      ignored: ["**/src-tauri/**"],
    },
  },
  build: {
    rollupOptions: {
      input: {
        main: resolve(__dirname, "index.html"),
        "subsystem-health": resolve(__dirname, "src/windows/subsystem-health/index.html"),
        "event-bus": resolve(__dirname, "src/windows/event-bus/index.html"),
        "chat": resolve(__dirname, "src/windows/chat/index.html"),
        "boot-timeline": resolve(__dirname, "src/windows/boot-timeline/index.html"),
        "toast": resolve(__dirname, "src/windows/toast/index.html"),
      },
    },
  },
}));
