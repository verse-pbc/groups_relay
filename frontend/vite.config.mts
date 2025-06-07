import { defineConfig } from "vite";

export default defineConfig({
  server: {
    host: true,
    cors: true,
  },
  build: {
    outDir: "dist",
    emptyOutDir: true,
    // Increase chunk size warning limit
    chunkSizeWarningLimit: 1000,
    rollupOptions: {
      onwarn(warning, warn) {
        // Suppress eval warnings from tseep
        if (warning.code === 'EVAL' && warning.id?.includes('tseep')) {
          return;
        }
        warn(warning);
      },
      output: {
        manualChunks: {
          // Split NDK and its dependencies into a separate chunk
          'ndk': [
            '@nostr-dev-kit/ndk',
            'nostr-tools',
            'tseep'
          ],
          // Split other large dependencies
          'vendor': [
            'preact',
            'localforage',
            '@cashu/cashu-ts'
          ]
        }
      }
    }
  },
});
