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
  build: {
    // Rollup's 500 kB default is calibrated for code shipped over a network,
    // where a large entry chunk costs download time on every cold visit. The
    // WebView loads this bundle from local disk out of the packaged app, so
    // that cost does not exist here. Splitting was measured and rejected: a
    // vendor/app split yields 292 kB + 375 kB, both eagerly loaded via
    // modulepreload before first paint, for 1 kB less total than the single
    // 667 kB chunk. It would drop both files under the threshold without
    // making startup any cheaper. The limit is raised rather than disabled so
    // genuine runaway growth still gets flagged.
    chunkSizeWarningLimit: 800,
  },
  // Tests reuse this config wholesale, so plugin/resolution behaviour in a
  // test is the same behaviour the app is built with.
  test: {
    environment: 'happy-dom',
    setupFiles: ['./src/test/setup.ts'],
    include: ['src/**/*.test.{ts,tsx}'],
  },
})
