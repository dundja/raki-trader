import { defineConfig } from '@playwright/test'

/**
 * Smoke tests against the real stack: `pnpm dev` on :3000 and traderd on :8787 (`make dev`).
 * One worker, because the control test changes the backend's shared trading state.
 *
 * An already running server on :3000 is reused (the documented `make dev` flow), but only if it
 * is this dashboard: e2e/smoke.spec.ts checks the page title before any test runs, so another
 * app that owns the port fails the run instead of being tested. `pnpm dev` itself uses
 * `--strictPort`: it refuses to start rather than silently moving to :3001, where the API's
 * single CORS origin would block every REST call and control.
 */
export default defineConfig({
  testDir: 'e2e',
  outputDir: 'test-results',
  workers: 1,
  reporter: 'list',
  use: { baseURL: 'http://localhost:3000', browserName: 'chromium' },
  webServer: {
    command: 'pnpm dev',
    url: 'http://localhost:3000',
    reuseExistingServer: true,
  },
})
