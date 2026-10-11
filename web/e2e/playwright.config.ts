import { defineConfig, devices } from '@playwright/test';

/**
 * End-to-end smoke tests: the production build, served with glave's Content Security Policy
 * (serve.mjs), in Chromium. Run `npm run build` first; `npm run e2e` runs these.
 */
export default defineConfig({
  testDir: '.',
  testMatch: '*.e2e.ts',
  fullyParallel: true,
  forbidOnly: process.env['CI'] !== undefined,
  retries: 0,
  reporter: [['list']],
  use: {
    baseURL: 'http://127.0.0.1:4310',
    trace: 'retain-on-failure',
  },
  projects: [{ name: 'chromium', use: { ...devices['Desktop Chrome'] } }],
  webServer: {
    command: 'node e2e/serve.mjs',
    url: 'http://127.0.0.1:4310/glaze',
    reuseExistingServer: false,
    cwd: '..',
  },
});
