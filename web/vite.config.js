import { resolve } from "node:path";
import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

export default defineConfig({
  base: "/admin/",
  plugins: [react()],
  publicDir: false,
  build: {
    outDir: "../crates/senixd/assets",
    emptyOutDir: false,
    rollupOptions: {
      input: { admin: resolve(import.meta.dirname, "admin.html") },
      output: {
        entryFileNames: "admin.js",
        chunkFileNames: "admin-[name].js",
        assetFileNames: "admin-[name][extname]"
      }
    }
  }
});
