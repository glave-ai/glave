import { readFileSync } from 'node:fs';
import { join } from 'node:path';

import AxeBuilder from '@axe-core/playwright';
import { type Locator, type Page, expect } from '@playwright/test';

/**
 * A glave the tests are served by (playwright.config.ts), known by the variables that hold the
 * address it serves on and its home. Each listens on a port it picks when it starts, so runs
 * side by side on one machine never meet; the run that starts them sets both variables, and its
 * workers read them.
 */
export interface Site {
  /** The variable that holds its address, as it said when it started. */
  readonly address: string;
  /** The variable that holds its home. */
  readonly home: string;
}

/** The app, its session API answered by each test's fake (fake-api.ts). */
export const APP: Site = { address: 'GLAVE_E2E_APP_URL', home: 'GLAVE_E2E_APP_HOME' };
/** Glaze's showcase. */
export const SHOWCASE: Site = {
  address: 'GLAVE_E2E_SHOWCASE_URL',
  home: 'GLAVE_E2E_SHOWCASE_HOME',
};
/** A glave with nothing set up, for the first run against the real server (first-run.e2e.ts). */
export const FIRST_RUN: Site = {
  address: 'GLAVE_E2E_FIRST_RUN_URL',
  home: 'GLAVE_E2E_FIRST_RUN_HOME',
};
/** A glave with a model, for signing in and connecting accounts (accounts.e2e.ts). */
export const ACCOUNTS: Site = {
  address: 'GLAVE_E2E_ACCOUNTS_URL',
  home: 'GLAVE_E2E_ACCOUNTS_HOME',
};
/** A glave reached from elsewhere, for passkeys (passkeys.e2e.ts). */
export const PASSKEYS: Site = {
  address: 'GLAVE_E2E_PASSKEYS_URL',
  home: 'GLAVE_E2E_PASSKEYS_HOME',
};
/**
 * A glave whose model is a fake service (fake-model.ts), asking before commands run, for
 * following a thread from two tabs (live.e2e.ts).
 */
export const LIVE: Site = { address: 'GLAVE_E2E_LIVE_URL', home: 'GLAVE_E2E_LIVE_HOME' };
/** The variable that holds the address of that glave's fake model service. */
export const LIVE_MODEL_URL = 'GLAVE_E2E_LIVE_MODEL_URL';
/** The variable that holds the run's lifeline, the socket each glave it starts ends with. */
export const LIFELINE = 'GLAVE_E2E_LIFELINE';
/**
 * The public origin the passkeys glave's settings name, which the tests route to it as a
 * tunnel would.
 */
export const PASSKEYS_ORIGIN = 'https://glave.test';

/** Where `site` serves, such as `http://127.0.0.1:52114`. */
export function served(site: Site): string {
  return kept(site.address);
}

/** The value the run keeps in `variable`; fails when it keeps none, outside a run. */
function kept(variable: string): string {
  const value = process.env[variable];
  if (value === undefined) {
    throw new Error(
      `${variable} is not set: run the tests with playwright.config.ts, which sets it`,
    );
  }
  return value;
}

/** The fragment of the link that signs the app in, with a code the fake API takes. */
export const SIGNED_IN = '#code=e2e-code';

/**
 * A sign-in link's one-time code from the real glave of `site`, asked for with its local token,
 * as `glave` alone asks for one.
 */
export async function signInCode(site: Site): Promise<string> {
  return codeFrom(site, 'signin/link');
}

/**
 * A setup link's code from the real glave of `site`, asked for with its local token, as
 * `glave server link` asks for one.
 */
export async function setupCode(site: Site): Promise<string> {
  return codeFrom(site, 'signin/setup');
}

/** The code `method` answers from the real glave of `site`. */
async function codeFrom(site: Site, method: string): Promise<string> {
  const token = readFileSync(join(kept(site.home), 'serve.token'), 'utf8').trim();
  const response = await fetch(`${served(site)}/`, {
    method: 'POST',
    headers: { Authorization: `Bearer ${token}`, 'Content-Type': 'application/json' },
    body: JSON.stringify({ jsonrpc: '2.0', id: 1, method, params: {} }),
  });
  const answer = (await response.json()) as { readonly result?: { readonly code?: unknown } };
  const code = answer.result?.code;
  if (typeof code !== 'string') {
    throw new Error(`glave gave no sign-in code: ${JSON.stringify(answer)}`);
  }
  return code;
}

/**
 * Makes the browser of `page` ask for `language` alone, as one set to it does. A context's
 * `locale` says so in Chromium, but WebKit, as Safari, takes only the languages the system
 * knows (`en-XA` reads `en-US` there), so the pages are told before any script of theirs runs.
 */
export async function readIn(page: Page, language: string): Promise<void> {
  await page.addInitScript((tag) => {
    Object.defineProperty(navigator, 'language', { get: () => tag });
    Object.defineProperty(navigator, 'languages', { get: () => [tag] });
  }, language);
}

/** Elements end-to-end tests use, by their `data-testid` (AGENTS.md, "Test hooks"). */
export function byTestId(page: Page, testId: string): ReturnType<Page['getByTestId']> {
  return page.getByTestId(testId);
}

/** How opaque an element shows: its own opacity times its ancestors'. */
export async function shownOpacity(locator: Locator): Promise<number> {
  return locator.evaluate((element) => {
    let opacity = 1;
    for (let node: Element | null = element; node !== null; node = node.parentElement) {
      opacity *= Number(getComputedStyle(node).opacity);
    }
    return opacity;
  });
}

/** A box on the screen, as Playwright measures it. */
interface Box {
  readonly x: number;
  readonly y: number;
  readonly width: number;
  readonly height: number;
}

/** How far a box may stick out of another, for sub-pixel rounding. */
const ROUNDING = 1;

/** Fails unless `inner` has a size and lies inside `outer`. */
function expectInside(name: string, inner: Box, outer: Box): void {
  const inside =
    inner.width > 0 &&
    inner.height > 0 &&
    inner.x >= outer.x - ROUNDING &&
    inner.y >= outer.y - ROUNDING &&
    inner.x + inner.width <= outer.x + outer.width + ROUNDING &&
    inner.y + inner.height <= outer.y + outer.height + ROUNDING;
  expect(inside, `${name} ${JSON.stringify(inner)} shows inside ${JSON.stringify(outer)}`).toBe(
    true,
  );
}

/** The box of the element with this `data-testid`; fails when it has none. */
async function boxOf(page: Page, testId: string): Promise<Box> {
  const element = byTestId(page, testId);
  await expect(element).toBeVisible();
  const box = await element.boundingBox();
  if (box === null) {
    throw new Error(`${testId} has no box`);
  }
  return box;
}

/**
 * Fails unless the dialog `testId` shows whole, measured by the boxes on the screen: the dialog
 * inside the viewport, and its close button (in its header), its body and each of `parts` (what
 * the body holds, the actions) inside the dialog, none of them empty. A dialog cut short hides
 * what overflows it, so a part outside its box is a part the person cannot see.
 */
export async function expectDialogShown(
  page: Page,
  testId: string,
  parts: readonly string[],
): Promise<void> {
  const size = page.viewportSize();
  if (size === null) {
    throw new Error('the page has no viewport');
  }
  const dialog = await boxOf(page, testId);
  expectInside(testId, dialog, { x: 0, y: 0, ...size });
  for (const part of [`${testId}-close`, `${testId}-body`, ...parts]) {
    expectInside(part, await boxOf(page, part), dialog);
  }
}

/** What a page says went wrong, as `watchForProblems` collects it. */
export interface Problems {
  /** Each thing it said, in its own words. */
  (): readonly string[];
  /**
   * Leaves the page for `url`, as `page.goto` does. Once a page is being left, WebKit refuses a
   * fetch it starts and says so, in words it uses for an access-control failure (Chromium lets
   * it through): the app cannot tell it is being left until it is gone. That line, for the
   * session API, said while this leaves the page, is not counted; anything else said then is.
   */
  leave(url: string): Promise<void>;
}

/**
 * What a page error says, whole. Playwright hands over an error split at its first colon, the
 * character after it dropped: WebKit's `Fetch API cannot load http://…` arrives as the name
 * `Fetch API cannot load http` and the message `/…`. A stack that begins with the name begins
 * with the text whole, frames after it, in both engines; without one, it is the name and the
 * message.
 */
function whole(error: Error): string {
  const [head = ''] = (error.stack ?? '').split('\n    at ');
  if (head !== '' && head.startsWith(error.name)) {
    return head.trimEnd();
  }
  return error.name === '' ? error.message : `${error.name}: ${error.message}`;
}

/**
 * Collects what the page says went wrong: errors on the console, uncaught errors and CSP
 * violations.
 */
export function watchForProblems(page: Page): Problems {
  const problems: string[] = [];
  const webkit = page.context().browser()?.browserType().name() === 'webkit';
  // What WebKit says of a fetch of the session API it refuses while this leaves the page.
  let refused: string | null = null;
  page.on('console', (message) => {
    if (message.type() === 'error') {
      problems.push(message.text());
    }
  });
  page.on('pageerror', (error) => {
    const said = whole(error);
    if (said !== refused) {
      problems.push(said);
    }
  });
  const leave = async (url: string): Promise<void> => {
    // The address the app's session API is called at (session.service.ts).
    const api = new URL('/', page.url()).href;
    refused = webkit ? `Fetch API cannot load ${api} due to access control checks.` : null;
    try {
      // The page is being left until the next one is committed: nothing of it runs after.
      await page.goto(url, { waitUntil: 'commit' });
    } finally {
      refused = null;
    }
    await page.waitForLoadState();
  };
  return Object.assign(() => problems, { leave });
}

/**
 * Fails on any WCAG 2.2 A or AA violation axe finds, colour contrast included. Colours are
 * measured once they stop changing: a button fades to its new background after a switch to
 * dark, and halfway through the fade its text would fail.
 */
export async function expectAccessible(page: Page): Promise<void> {
  await page.evaluate(() =>
    Promise.all(
      document
        .getAnimations()
        .filter((animation) => animation instanceof CSSTransition)
        .map((transition) => transition.finished.catch(() => undefined)),
    ),
  );
  const results = await new AxeBuilder({ page })
    .withTags(['wcag2a', 'wcag2aa', 'wcag21a', 'wcag21aa', 'wcag22aa'])
    .analyze();
  const found = results.violations.map(
    (violation) =>
      `${violation.id}: ${violation.help} (${violation.nodes.map((node) => node.target.join(' ')).join(', ')})`,
  );
  expect(found).toEqual([]);
}
