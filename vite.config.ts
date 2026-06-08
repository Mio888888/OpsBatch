import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// @ts-expect-error process is a nodejs global
const host = process.env.TAURI_DEV_HOST;

function vendorChunk(id: string) {
  if (!id.includes('/node_modules/')) return undefined;

  if (id.includes('/node_modules/react/') || id.includes('/node_modules/react-dom/') || id.includes('/node_modules/scheduler/')) {
    return 'react';
  }

  if (id.includes('/node_modules/react-router-dom/') || id.includes('/node_modules/react-router/')) {
    return 'router';
  }

  if (id.includes('/node_modules/@tauri-apps/')) {
    return 'tauri';
  }

  if (id.includes('/node_modules/@codemirror/lang-') || id.includes('/node_modules/@codemirror/legacy-modes/') || id.includes('/node_modules/@lezer/')) {
    return undefined;
  }

  if (id.includes('/node_modules/@codemirror/') || id.includes('/node_modules/@uiw/') || id.includes('/node_modules/style-mod/') || id.includes('/node_modules/w3c-keyname/')) {
    return 'codemirror-core';
  }

  if (id.includes('/node_modules/@xterm/')) {
    return 'xterm';
  }

  if (id.includes('/node_modules/@xyflow/')) {
    return 'xyflow';
  }

  if (id.includes('/node_modules/@radix-ui/') || id.includes('/node_modules/treeselectjs/')) {
    return 'ui-vendor';
  }

  if (id.includes('/node_modules/@dnd-kit/') || id.includes('/node_modules/gsap/')) {
    return 'interaction-vendor';
  }

  if (id.includes('/node_modules/lucide-react/')) {
    return 'icons';
  }

  return 'vendor';
}

// https://vite.dev/config/
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
      output: {
        manualChunks: vendorChunk,
      },
    },
  },
}));
