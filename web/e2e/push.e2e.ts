import { THREAD, fakeApi } from './fake-api';
import { fakePush } from './fake-push';
import {
  PUSH,
  SIGNED_IN,
  byTestId,
  expect,
  expectAccessible,
  overlaps,
  served,
  signInCode,
  test,
} from './support';

/**
 * Where the browser's own push service would take this browser's pushes, and its keys (RFC
 * 8291's example browser's), as the test stands them in: what a real glave takes and keeps.
 */
const SUBSCRIPTION = {
  endpoint: 'https://fcm.googleapis.com/fcm/send/e2e-browser',
  p256dh: 'BCVxsr7N_eNgVRqvHtD0zTZsEc6-VV-JvLexhqUzORcxaOzi6-AYWXvTBHm4bjyPjs7Vd8pZGH6SRpkNtoIAiw4',
  auth: 'BTBZMqHH6r4Tts7J_aSIgg',
} as const;

/** The page's window, with how many times the browser asked the person. */
type Asking = Window & { asked?: number };

/**
 * Stands in for what a headless browser lacks: a person not yet asked about notifications (a
 * headless Chromium says they refused), who says yes when asked, counted in `asked`; and the
 * browser's push service, which a headless browser does not reach, so a subscription is made at
 * once, as `subscription`, and nothing leaves the computer. Where the browser has no Push API,
 * as Playwright's WebKit has none, it stands in the API's names too, so the page offers
 * notifications as Safari does. The page's own script, its service worker and the server's
 * Content Security Policy are the real ones.
 */
function standIns(subscription: typeof SUBSCRIPTION): void {
  const page = window as Asking;
  page.asked = 0;
  const missing = (name: string, value: unknown): void => {
    if (!(name in window)) {
      Object.defineProperty(window, name, { configurable: true, writable: true, value });
    }
  };
  // Constructors whose methods are stood in below, as the browser's own would be.
  missing('PushManager', function PushManager() {
    // Made by the browser alone.
  });
  missing('Notification', function Notification() {
    // Shown by the browser alone.
  });
  if (!('serviceWorker' in navigator)) {
    const workers = Object.assign(new EventTarget(), {
      getRegistration: () => Promise.resolve(undefined),
      register: () => Promise.reject(new Error('no service workers here')),
    });
    Object.defineProperty(navigator, 'serviceWorker', { configurable: true, value: workers });
  }
  let permission: NotificationPermission = 'default';
  Object.defineProperty(Notification, 'permission', { configurable: true, get: () => permission });
  Notification.requestPermission = () => {
    page.asked = (page.asked ?? 0) + 1;
    permission = 'granted';
    return Promise.resolve(permission);
  };
  // A subscription lasts across the tab's pages, as the browser keeps one.
  const kept = 'e2e-subscribed';
  const { endpoint, p256dh, auth } = subscription;
  const made = {
    endpoint,
    options: { applicationServerKey: null, userVisibleOnly: true },
    toJSON: () => ({ endpoint, keys: { p256dh, auth } }),
    unsubscribe: () => {
      sessionStorage.removeItem(kept);
      return Promise.resolve(true);
    },
  } as unknown as PushSubscription;
  PushManager.prototype.subscribe = function subscribe() {
    sessionStorage.setItem(kept, 'yes');
    return Promise.resolve(made);
  };
  PushManager.prototype.getSubscription = function getSubscription() {
    return Promise.resolve(sessionStorage.getItem(kept) === null ? null : made);
  };
}

for (const colorScheme of ['light', 'dark'] as const) {
  test(`a thread offers notifications above its first lines, asking nothing until Turn on, and Not now is remembered (${colorScheme})`, async ({
    page,
  }) => {
    await page.emulateMedia({ colorScheme });
    await page.addInitScript(standIns, SUBSCRIPTION);
    await fakeApi(page);
    const push = await fakePush(page);
    await page.goto(`/t/${THREAD}${SIGNED_IN}`);
    const banner = byTestId(page, 'push-banner');
    const first = byTestId(page, 'thread-items').locator('app-turn').first();
    await expect(banner).toContainText(
      'Get a notification when glave needs you, even with this tab closed.',
    );
    await expect(first).toBeVisible();
    // The page asked the browser nothing as it opened, and the offer covers none of the thread.
    expect(await page.evaluate(() => (window as Asking).asked)).toBe(0);
    expect(overlaps(await banner.boundingBox(), await first.boundingBox())).toBe(false);
    await expectAccessible(page);
    // Going, it keeps its room: nothing moves.
    const before = await first.boundingBox();
    await byTestId(page, 'push-banner-not-now').click();
    await expect(banner).toBeHidden();
    expect(await first.boundingBox()).toEqual(before);
    await page.reload();
    await expect(first).toBeVisible();
    await expect(banner).toHaveCount(0);
    expect(await page.evaluate(() => (window as Asking).asked)).toBe(0);
    expect(push.calls).not.toContain('push/subscribe');
  });
}

test('Turn on registers glave’s worker under its own policy, and the real server keeps this browser', async ({
  page,
  browserName,
}) => {
  test.skip(
    browserName === 'webkit',
    "Playwright's WebKit has no push service to subscribe through",
  );
  await page.addInitScript(standIns, SUBSCRIPTION);
  const glave = served(PUSH);
  await page.goto(`${glave}/#code=${await signInCode(PUSH)}`);
  await expect(byTestId(page, 'home-composer-input')).toBeVisible();
  await page.goto(`${glave}/settings`);
  const state = byTestId(page, 'notifications-state');
  await expect(state).toHaveText('This browser doesn’t get notifications.');
  await byTestId(page, 'notifications-switch').click();
  await expect(state).toHaveText('This browser gets notifications.');
  expect(await page.evaluate(() => (window as Asking).asked)).toBe(1);
  // The worker the server's CSP and Trusted Types let the page register, running.
  const worker = (): Promise<string | null> =>
    page.evaluate(async () => {
      const registration = await navigator.serviceWorker.getRegistration('/');
      return registration?.active?.scriptURL ?? null;
    });
  await expect.poll(worker).toMatch(/\/push-worker\.js\?fallback=/u);
  // The server kept it: read again, this browser is still the one it pushes to.
  await page.reload();
  await expect(state).toHaveText('This browser gets notifications.');
  await expect(byTestId(page, 'notifications-device')).toContainText('This browser');
  await expectAccessible(page);
  // Off again, the server forgets it.
  await byTestId(page, 'notifications-switch').click();
  await expect(state).toHaveText('This browser doesn’t get notifications.');
  await page.reload();
  await expect(state).toHaveText('This browser doesn’t get notifications.');
  await expect(byTestId(page, 'notifications-device')).toHaveCount(0);
});
