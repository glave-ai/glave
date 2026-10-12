import { mkdtempSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';

import { defineConfig, devices } from '@playwright/test';

import { APP_URL, SHOWCASE_URL } from './support';

/** The glave binary that serves the app: GLAVE_BIN, else the workspace's debug build. */
const glave = process.env['GLAVE_BIN'] ?? resolve(import.meta.dirname, '../../target/debug/glave');

/** What glave serves, and where: the app, and Glaze's showcase beside it. */
const SITES = [
  { url: APP_URL, web: 'dist/app/browser' },
  { url: SHOWCASE_URL, web: 'dist/showcase/browser' },
];

/**
 * End-to-end smoke tests: the production builds (`npm run build`, `npm run build:showcase`)
 * served by the real `glave serve --http --web`, with glave's own Content Security Policy and
 * headers, in Chromium. The session API's calls are answered by each test's fake
 * (fake-api.ts), so no model is needed. Build glave first (`cargo build -p glave`); `cargo
 * xtask ci` does.
 */
export default defineConfig({
  testDir: '.',
  testMatch: '*.e2e.ts',
  fullyParallel: true,
  forbidOnly: process.env['CI'] !== undefined,
  retries: 0,
  reporter: [['list']],
  use: {
    baseURL: APP_URL,
    trace: 'retain-on-failure',
  },
  projects: [{ name: 'chromium', use: { ...devices['Desktop Chrome'] } }],
  // Each from a glave home of its own, so the tests touch nothing of the person's.
  webServer: SITES.map(({ url, web }) => ({
    command: `"${glave}" serve --http ${new URL(url).port} --web ${web}`,
    url,
    reuseExistingServer: false,
    cwd: '..',
    env: { GLAVE_HOME: mkdtempSync(join(tmpdir(), 'glave-e2e-')), GLAVE_MODEL: 'e2e' },
  })),
});
