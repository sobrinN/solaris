import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";

export default defineConfig({
  plugins: [react(), tailwindcss()],
  server: {
    port: 5173,
    strictPort: true,
  },
  build: {
    rollupOptions: {
      output: {
        manualChunks(id) {
          const normalized = id.replace(/\\/g, "/");
          if (
            normalized.includes("/node_modules/react/") ||
            normalized.includes("/node_modules/react-dom/") ||
            normalized.includes("/node_modules/scheduler/")
          ) {
            return "react-vendor";
          }
          if (normalized.includes("/node_modules/lucide-react/")) {
            return "lucide-icons";
          }
          if (
            normalized.includes("@solaris/gfx-runtime") ||
            normalized.includes("/packages/gfx-runtime/") ||
            normalized.includes("/node_modules/gsap/")
          ) {
            return "gfx-runtime";
          }
          return undefined;
        },
      },
    },
  },
});
