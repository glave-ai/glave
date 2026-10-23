import { mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { type Socket, createServer } from 'node:net';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';

import { defineConfig, devices } from '@playwright/test';

import { LIVE_MODEL, startFakeModel } from './fake-model';
import {
  ACCOUNTS,
  APP,
  FIRST_RUN,
  HERE,
  HERE_WEBKIT,
  LIFELINE,
  LIVE,
  LIVE_MODEL_URL,
  PASSKEYS,
  PASSKEYS_ORIGIN,
  SHOWCASE,
  type Site,
} from './support';

/** The glave binary that serves the app: GLAVE_BIN, else the workspace's debug build. */
const glave = process.env['GLAVE_BIN'] ?? resolve(import.meta.dirname, '../../target/debug/glave');

/**
 * A home of its own for the glave of `site`, with `settings` as its config.toml, so the tests
 * touch nothing of the person's: made once, in the run that starts the servers, whose workers
 * read this file again with its variable set.
 */
function home(site: Site, settings?: string): string {
  const kept = process.env[site.home];
  if (kept !== undefined) {
    return kept;
  }
  const made = mkdtempSync(join(tmpdir(), 'glave-e2e-'));
  if (settings !== undefined) {
    writeFileSync(join(made, 'config.toml'), settings);
  }
  process.env[site.home] = made;
  return made;
}

/**
 * The socket every glave the run starts is handed (`GLAVE_LIFELINE`), so they end when the run
 * does, however it ends, killed outright included (#558): each glave connects and waits, and
 * the connections close when the run that listens ends. Made once, in the run that starts the
 * servers, which alone listens, and removed when that run exits; its workers read its path
 * again from the variable.
 */
function lifeline(): string {
  const kept = process.env[LIFELINE];
  if (kept !== undefined) {
    return kept;
  }
  const folder = mkdtempSync(join(tmpdir(), 'glave-e2e-lifeline-'));
  const path = join(folder, 'lifeline');
  const held: Socket[] = [];
  // Neither the listener nor a connection keeps the run from ending.
  createServer((connection) => {
    connection.unref();
    held.push(connection);
  })
    .listen(path)
    .unref();
  process.on('exit', () => {
    rmSync(folder, { recursive: true, force: true });
  });
  process.env[LIFELINE] = path;
  return path;
}

/** The run's lifeline, handed to each glave it starts. */
const tied = { GLAVE_LIFELINE: lifeline() };

/**
 * The fake model service the live glave asks, started once, in the run that starts the servers,
 * on a port the system picks; its workers read its address again from the variable.
 */
const liveModel = process.env[LIVE_MODEL_URL] ?? (await startFakeModel());
process.env[LIVE_MODEL_URL] = liveModel;

/**
 * What glave serves: the app on a model the fake API names, Glaze's showcase, the app on a
 * glave with nothing set up, for the first run against the real server, the app on a real
 * glave with a model, to connect accounts to, the app on a real glave reached from elsewhere
 * at a public origin, to sign in to with passkeys, the app on real glaves reached here alone,
 * to make passkeys on this computer, and the app on a real glave whose model is a fake service
 * and which asks before commands run, to follow a thread from two tabs.
 */
const SITES: readonly { site: Site; web: string; env: Record<string, string> }[] = [
  {
    site: APP,
    web: 'dist/app/browser',
    env: { ...tied, GLAVE_HOME: home(APP), GLAVE_MODEL: 'e2e' },
  },
  {
    site: SHOWCASE,
    web: 'dist/showcase/browser',
    env: { ...tied, GLAVE_HOME: home(SHOWCASE), GLAVE_MODEL: 'e2e' },
  },
  {
    site: FIRST_RUN,
    web: 'dist/app/browser',
    env: { ...tied, GLAVE_HOME: home(FIRST_RUN), GLAVE_MODEL: '' },
  },
  {
    site: ACCOUNTS,
    web: 'dist/app/browser',
    env: { ...tied, GLAVE_HOME: home(ACCOUNTS), GLAVE_MODEL: 'e2e' },
  },
  ...[HERE, HERE_WEBKIT].map((site) => ({
    site,
    web: 'dist/app/browser',
    env: { ...tied, GLAVE_HOME: home(site), GLAVE_MODEL: 'e2e' },
  })),
  {
    site: PASSKEYS,
    web: 'dist/app/browser',
    env: {
      ...tied,
      GLAVE_HOME: home(PASSKEYS, `[serve]\norigin = "${PASSKEYS_ORIGIN}"\n`),
      GLAVE_MODEL: 'e2e',
    },
  },
  {
    site: LIVE,
    web: 'dist/app/browser',
    env: {
      ...tied,
      GLAVE_HOME: home(LIVE, 'approval = "ask"\n'),
      GLAVE_BASE_URL: liveModel,
      GLAVE_MODEL: LIVE_MODEL,
    },
  },
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
    // Read in the workers, which start once every glave has said where it serves.
    baseURL: process.env[APP.address],
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
  // Each glave listens on a port the system picks (port 0), so two runs on one machine, such as
  // two `cargo xtask ci` side by side, never meet. It is ready once it says where it serves,
  // and Playwright keeps that address in the site's variable, which the workers inherit.
  webServer: SITES.map(({ site, web, env }) => ({
    command: `"${glave}" serve --http 0 --web ${web}`,
    wait: { stderr: new RegExp(`serving the session API on (?<${site.address}>http://\\S+)`, 'u') },
    cwd: '..',
    env,
  })),
});
