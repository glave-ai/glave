import { mkdtempSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';

import { defineConfig, devices } from '@playwright/test';

/** The glave binary that serves the app: GLAVE_BIN, else the workspace's debug build. */
const glave = process.env['GLAVE_BIN'] ?? resolve(import.meta.dirname, '../../target/debug/glave');

/** A glave home of the run's own, so the tests touch nothing of the person's. */
const home = mkdtempSync(join(tmpdir(), 'glave-e2e-'));

/**
 * End-to-end smoke tests: the production build (`npm run build`) served by the real
 * `glave serve --http --web`, with glave's own Content Security Policy and headers, in
 * Chromium. The session API's calls are answered by each test's fake (fake-api.ts), so no
 * model is needed. Build glave first (`cargo build -p glave`); `cargo xtask ci` does.
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
    command: `"${glave}" serve --http 4310 --web dist/app/browser`,
    url: 'http://127.0.0.1:4310/glaze',
    reuseExistingServer: false,
    cwd: '..',
    env: { GLAVE_HOME: home, GLAVE_MODEL: 'e2e' },
  },
});
