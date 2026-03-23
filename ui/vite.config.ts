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
        "toast": resolve(__dirname, "src/windows/toast/index.html"),
        "notification-history": resolve(__dirname, "src/windows/notification-history/index.html"),
        "vignette": resolve(__dirname, "src/windows/vignette/index.html"),
        "resources": resolve(__dirname, "src/windows/resources/index.html"),
        "thought-stream": resolve(__dirname, "src/windows/thought-stream/index.html"),
        "memory-stats": resolve(__dirname, "src/windows/memory-stats/index.html"),
        "prompt-trace": resolve(__dirname, "src/windows/prompt-trace/index.html"),
        "conversation-timeline": resolve(__dirname, "src/windows/conversation-timeline/index.html"),
        "widget-bar": resolve(__dirname, "src/windows/widget-bar/index.html"),
        "settings": resolve(__dirname, "src/windows/settings/index.html"),
        "model-panel": resolve(__dirname, "src/windows/model-panel/index.html"),
      },
    },
  },
}));
