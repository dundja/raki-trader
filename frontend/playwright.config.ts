import { defineConfig } from '@playwright/test'

/**
 * Smoke tests against the real stack: `pnpm dev` on :3000 and traderd on :8787 (`make dev`).
 * One worker, because the control test changes the backend's shared trading state.
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
