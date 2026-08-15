/// <reference types="vitest/config" />
import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'
import tailwindcss from '@tailwindcss/vite'

export default defineConfig({
  plugins: [react(), tailwindcss()],
  server: {
    port: 1420,
    strictPort: true,
  },
  optimizeDeps: {
    // `bindings.ts` reaches this subpath through an inline `import { type ... }`,
    // which the cold-start dependency scan drops. Vite therefore discovers it
    // only when the browser first requests it, re-optimizes, and broadcasts a
    // full page reload — a reload the very first navigation can race and lose.
    // Naming it here puts it in the initial optimized set instead.
    include: ["@tauri-apps/api/webviewWindow"],
  },
  clearScreen: false,
  // Tests reuse this config wholesale, so plugin/resolution behaviour in a
  // test is the same behaviour the app is built with.
  test: {
    environment: 'happy-dom',
    setupFiles: ['./src/test/setup.ts'],
    include: ['src/**/*.test.{ts,tsx}'],
  },
})
