import { mkdtempSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';

import { defineConfig, devices } from '@playwright/test';

import {
  ACCOUNTS_HOME,
  ACCOUNTS_URL,
  APP_URL,
  FIRST_RUN_HOME,
  FIRST_RUN_URL,
  PASSKEYS_HOME,
  PASSKEYS_ORIGIN,
  PASSKEYS_URL,
  SHOWCASE_URL,
} from './support';

/** The glave binary that serves the app: GLAVE_BIN, else the workspace's debug build. */
const glave = process.env['GLAVE_BIN'] ?? resolve(import.meta.dirname, '../../target/debug/glave');

/** A home of its own for each glave, so the tests touch nothing of the person's. */
function home(): string {
  return mkdtempSync(join(tmpdir(), 'glave-e2e-'));
}

/**
 * The first run's home, which its test reads the sign-in token from: made once, in the run
 * that starts the servers, whose workers read this file again with the variable set.
 */
const firstRun = process.env[FIRST_RUN_HOME] ?? home();
process.env[FIRST_RUN_HOME] = firstRun;
/** The home of the glave that accounts are connected to, kept the same way. */
const accounts = process.env[ACCOUNTS_HOME] ?? home();
process.env[ACCOUNTS_HOME] = accounts;
/** The home of the glave reached from elsewhere, its settings naming its public origin. */
const passkeys =
  process.env[PASSKEYS_HOME] ??
  ((): string => {
    const made = home();
    writeFileSync(join(made, 'config.toml'), `[serve]\norigin = "${PASSKEYS_ORIGIN}"\n`);
    return made;
  })();
process.env[PASSKEYS_HOME] = passkeys;

/**
 * What glave serves, and where: the app on a model the fake API names, Glaze's showcase, the
 * app on a glave with nothing set up, for the first run against the real server, the app on a
 * real glave with a model, to connect accounts to, and the app on a real glave reached from
 * elsewhere at a public origin, to sign in to with passkeys.
 */
const SITES = [
  { url: APP_URL, web: 'dist/app/browser', env: { GLAVE_HOME: home(), GLAVE_MODEL: 'e2e' } },
  {
    url: SHOWCASE_URL,
    web: 'dist/showcase/browser',
    env: { GLAVE_HOME: home(), GLAVE_MODEL: 'e2e' },
  },
  { url: FIRST_RUN_URL, web: 'dist/app/browser', env: { GLAVE_HOME: firstRun, GLAVE_MODEL: '' } },
  { url: ACCOUNTS_URL, web: 'dist/app/browser', env: { GLAVE_HOME: accounts, GLAVE_MODEL: 'e2e' } },
  { url: PASSKEYS_URL, web: 'dist/app/browser', env: { GLAVE_HOME: passkeys, GLAVE_MODEL: 'e2e' } },
];

/**
 * End-to-end smoke tests: the production builds (`npm run build`, `npm run build:showcase`)
 * served by the real `glave serve --http --web`, with glave's own Content Security Policy and
 * headers, in Chromium and in WebKit, Safari's engine, headless. Every test runs in both, so a
 * screen Safari alone gets wrong fails here first; the few that skip WebKit say why. The
 * session API's calls are answered by each test's fake (fake-api.ts), so no model is needed;
 * the first run's are the real server's, with a fake model service (first-run.e2e.ts). Build
 * glave first (`cargo build -p glave`); `cargo xtask ci` does.
 */
export default defineConfig({
  testDir: '.',
  testMatch: '*.e2e.ts',
  fullyParallel: true,
  // As many at once as GLAVE_TEST_THREADS says (`cargo xtask ci` sets it on a laptop), else
  // Playwright's own choice.
  workers: Number(process.env['GLAVE_TEST_THREADS']) || undefined,
  forbidOnly: process.env['CI'] !== undefined,
  retries: 0,
  // Time limits are guards against a hang, not speed limits, as for the unit tests
  // (vitest.config.mjs) and nextest: a page that answers in a second on an idle machine can
  // take twenty on a laptop busy with other builds. A test gets 3 minutes, each expectation 30 s.
  timeout: 180_000,
  expect: { timeout: 30_000 },
  reporter: [['list']],
  use: {
    baseURL: APP_URL,
    trace: 'retain-on-failure',
  },
  projects: [
    { name: 'chromium', use: { ...devices['Desktop Chrome'] } },
    {
      name: 'webkit',
      use: { ...devices['Desktop Safari'] },
      // These two change a real glave (its first model, a connected account), which the
      // Chromium run owns: run twice, they would race on its state.
      testIgnore: ['first-run.e2e.ts', 'accounts.e2e.ts'],
    },
  ],
  webServer: SITES.map(({ url, web, env }) => ({
    command: `"${glave}" serve --http ${new URL(url).port} --web ${web}`,
    url,
    reuseExistingServer: false,
    cwd: '..',
    env,
  })),
});
