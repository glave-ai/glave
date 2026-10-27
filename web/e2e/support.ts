import { readFileSync, readdirSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';

import type { Method } from '@glave/client';
import { type BrowserContext, type Locator, type Page, expect } from '@playwright/test';
import type { Result, default as axe } from 'axe-core';

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
/**
 * Glaves reached on this computer alone, for passkeys made here, one for each engine, since
 * each makes the first (passkeys.e2e.ts).
 */
export const HERE: Site = { address: 'GLAVE_E2E_HERE_URL', home: 'GLAVE_E2E_HERE_HOME' };
export const HERE_WEBKIT: Site = {
  address: 'GLAVE_E2E_HERE_WEBKIT_URL',
  home: 'GLAVE_E2E_HERE_WEBKIT_HOME',
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
/**
 * Glaves whose person has a service of their own to sign in to, one for each engine, since each
 * connects it and disconnects it (connections.e2e.ts).
 */
export const CONNECTIONS: Site = {
  address: 'GLAVE_E2E_CONNECTIONS_URL',
  home: 'GLAVE_E2E_CONNECTIONS_HOME',
};
export const CONNECTIONS_WEBKIT: Site = {
  address: 'GLAVE_E2E_CONNECTIONS_WEBKIT_URL',
  home: 'GLAVE_E2E_CONNECTIONS_WEBKIT_HOME',
};
/** The glave binary that serves the app: GLAVE_BIN, else the workspace's debug build. */
export const GLAVE_BIN =
  process.env['GLAVE_BIN'] ?? resolve(import.meta.dirname, '../../target/debug/glave');
/**
 * The MCP test server glave's own tests run, built beside glave (glave-mcp's `test-server`): a
 * service that asks for a sign-in, at a fake authorization server of its own.
 */
export const TEST_SERVER = join(dirname(GLAVE_BIN), 'glave-mcp-test-server');
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

/** The home of the glave of `site`, which holds the person's settings. */
export function homeOf(site: Site): string {
  return kept(site.home);
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
  const token = readFileSync(join(homeOf(site), 'serve.token'), 'utf8').trim();
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

/**
 * `context`, every request of its pages held until Playwright has heard the page make it, as the
 * fake API's routes hold them, so no tab of it is stranded on its first load of a real glave. A
 * tab of its own (`context.newPage()`) starts on a page of no origin, and glave's
 * `Cross-Origin-Opener-Policy: same-origin` moves its first load of glave to a new process. Linux
 * WebKit cancels that load, and never commits it, when the new process gets glave's answer before
 * WebKit has heard from the old one that the load began: the tab stays on `about:blank` and
 * `goto` waits until the test's time runs out (WebKit bug 325840, microsoft/playwright#42957).
 * The old process says the load began before it makes the request, and Playwright hears the
 * request from it the same way, so a request held until Playwright has heard it is answered only
 * after WebKit has heard that the load began.
 */
export async function heldUntilHeard(context: BrowserContext): Promise<BrowserContext> {
  await context.route('**/*', (route) => route.fallback());
  return context;
}

/**
 * A tab beside `from`, opened from it on a blank page, for the test to go to glave in. It starts
 * in `from`'s origin and browsing context group, which glave's `Cross-Origin-Opener-Policy:
 * same-origin` keeps it in, so its load stays in `from`'s process and moves to no new one (a
 * tab of its own needs `heldUntilHeard`). Like any tab opened from another, it starts with a
 * copy of `from`'s session storage.
 */
export async function newTab(from: Page): Promise<Page> {
  const [tab] = await Promise.all([
    from.waitForEvent('popup'),
    from.evaluate(() => {
      window.open('about:blank');
    }),
  ]);
  return tab;
}

/**
 * The file of the app's build, as glave serves it, whose code `holds` picks out, such as the
 * chunk with a part the app loads late, for a test to hold on its way. Fails when none does.
 */
export function builtChunk(holds: (code: string) => boolean): string {
  const built = new URL('../dist/app/browser/', import.meta.url);
  const found = readdirSync(built).find(
    (file) => file.endsWith('.js') && holds(readFileSync(new URL(file, built), 'utf8')),
  );
  if (found === undefined) {
    throw new Error('no file of the app’s build holds what the test looks for: npm run build');
  }
  return found;
}

/** What a test holds on its way (`hold`) until it lets it go. */
export interface Held {
  /** Settles once the page has sent the first of it. */
  readonly sent: Promise<void>;
  /** Lets it go on to what answers it, and holds nothing after. */
  readonly letGo: () => void;
}

/**
 * Holds what `on` (a page, or every page of a context) sends to `url` (a pattern, as `route`
 * takes), or only its calls of the session API's `method` there, on its way until the test lets
 * it go, as a busy server or a slow network would: so the test sees what the page shows
 * meanwhile for as long as it looks, with no timer to race. A route added later runs first, so
 * called after `fakeApi`, whose route answers every call itself, this holds a call before the
 * fake answers it; with no fake, it holds a request before the glave, or the service, does.
 */
export async function hold(
  on: Page | BrowserContext,
  url: Parameters<Page['route']>[0],
  method?: Method,
): Promise<Held> {
  let sending = (): void => undefined;
  const sent = new Promise<void>((resolve) => {
    sending = resolve;
  });
  let letGo = (): void => undefined;
  const held = new Promise<void>((resolve) => {
    letGo = resolve;
  });
  await on.route(url, async (route) => {
    if (method === undefined || route.request().headers()['glave-method'] === method) {
      sending();
      await held;
    }
    await route.fallback();
  });
  return { sent, letGo };
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

/**
 * How far apart the space above and below may be in `expectEven`: Chromium rounds the space
 * above a line's letters down to a whole pixel and adds what it took to the space below, so the
 * two differ by up to two pixels in a line that is even.
 */
const EVEN = 2;

/** The top and bottom of something on the screen. */
interface Edges {
  readonly top: number;
  readonly bottom: number;
}

/** The edges of the box `locator` finds. */
async function edgesOf(locator: Locator): Promise<Edges> {
  return locator.evaluate((element) => {
    const { top, bottom } = element.getBoundingClientRect();
    return { top, bottom };
  });
}

/**
 * The edges of `box` and of the letters of the text it shows (blank lines and spaces at a
 * text's ends draw nothing), measured at once; `held` is null when it shows no text.
 */
async function lettersIn(box: Locator): Promise<{ outer: Edges; held: Edges | null }> {
  return box.evaluate((element) => {
    const letters: DOMRect[] = [];
    const walker = document.createTreeWalker(element, NodeFilter.SHOW_TEXT);
    for (let node = walker.nextNode(); node !== null; node = walker.nextNode()) {
      const text = node.textContent ?? '';
      const start = text.search(/\S/u);
      if (start >= 0 && node.parentElement?.checkVisibility() === true) {
        const range = document.createRange();
        range.setStart(node, start);
        range.setEnd(node, text.trimEnd().length);
        letters.push(...range.getClientRects());
      }
    }
    const { top, bottom } = element.getBoundingClientRect();
    const held =
      letters.length === 0
        ? null
        : {
            top: Math.min(...letters.map((line) => line.top)),
            bottom: Math.max(...letters.map((line) => line.bottom)),
          };
    return { outer: { top, bottom }, held };
  });
}

/**
 * Fails unless `box` holds what it shows evenly, so a rule beside it or a fill under it ends
 * where what it holds does (#610): as much space inside it above the first line of its text as
 * below the last, or, given `items`, above the first of them as below the last.
 */
export async function expectEven(box: Locator, items?: Locator): Promise<void> {
  const { outer, held } =
    items === undefined
      ? await lettersIn(box)
      : {
          outer: await edgesOf(box),
          held: {
            top: (await edgesOf(items.first())).top,
            bottom: (await edgesOf(items.last())).bottom,
          },
        };
  if (held === null) {
    throw new Error('the box measured for evenness shows no text');
  }
  const space = { above: held.top - outer.top, below: outer.bottom - held.bottom };
  expect(Math.abs(space.above - space.below), JSON.stringify(space)).toBeLessThan(EVEN);
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

/** The page's window once `withAxe` has put axe in it. */
export type WithAxe = Window & { readonly axe: typeof axe };

/**
 * Puts axe in `page`, from axe-core's own source: the one axe, in the one version, that every
 * check runs, the unit tests' too. Its module is read only here, as a test runs: as it loads it
 * reads a page's window, which the config, read with this module in Node alone, has none of.
 */
export async function withAxe(page: Page): Promise<void> {
  const { source } = (await import('axe-core')).default;
  await page.evaluate(source);
}

/** The rules axe holds the pages to: WCAG 2.2, levels A and AA. */
export const WCAG: readonly string[] = ['wcag2a', 'wcag2aa', 'wcag21a', 'wcag21aa', 'wcag22aa'];

/** Each violation axe found, as a line: its rule, what it asks and where it failed. */
export function described(violations: readonly Result[]): readonly string[] {
  return violations.map(
    (violation) =>
      `${violation.id}: ${violation.help} (${violation.nodes.map((node) => node.target.join(' ')).join(', ')})`,
  );
}

/**
 * Fails on any WCAG 2.2 A or AA violation axe finds, colour contrast included. Colours are
 * measured once they stop changing: a button fades to its new background after a switch to
 * dark, and a reply fades in as it arrives, and halfway through either its text would fail.
 * What loops while work goes on (a shimmer, a turning ring) never stops, so it runs on.
 */
export async function expectAccessible(page: Page): Promise<void> {
  await page.evaluate(() =>
    Promise.all(
      document
        .getAnimations()
        .filter((animation) => animation.effect?.getComputedTiming().iterations !== Infinity)
        .map((animation) => animation.finished.catch(() => undefined)),
    ),
  );
  await withAxe(page);
  const violations = await page.evaluate(async (tags) => {
    const results = await (window as unknown as WithAxe).axe.run(document, {
      runOnly: { type: 'tag', values: [...tags] },
    });
    return results.violations;
  }, WCAG);
  expect(described(violations)).toEqual([]);
}
