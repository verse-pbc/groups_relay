import { defineConfig } from "vite";

export default defineConfig({
  server: {
    host: true,
    cors: true,
  },
  build: {
    outDir: "dist",
    emptyOutDir: true,
  },
});
