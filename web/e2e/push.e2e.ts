import { THREAD, fakeApi } from './fake-api';
import { SIGNED_IN, byTestId, expect, expectAccessible, test } from './support';

/** Where the browser's own push service would take this browser's pushes, as the test stands it in. */
const ENDPOINT = 'https://push.example.net/e2e';

/** The page's window, with how many times the browser asked the person. */
type Asking = Window & { asked?: number };

/**
 * Stands in for what a headless browser lacks: a person not yet asked about notifications (a
 * headless Chromium says they refused), who says yes when asked, counted in `asked`; and the
 * browser's push service, which a headless browser does not reach, so a subscription is made at
 * once, at `endpoint`, and nothing leaves the computer. The page's own script, its service worker
 * and the server's Content Security Policy are the real ones.
 */
function standIns(endpoint: string): void {
  const page = window as Asking;
  page.asked = 0;
  let permission: NotificationPermission = 'default';
  Object.defineProperty(Notification, 'permission', { configurable: true, get: () => permission });
  Notification.requestPermission = () => {
    page.asked = (page.asked ?? 0) + 1;
    permission = 'granted';
    return Promise.resolve(permission);
  };
  // A subscription lasts across the tab's pages, as the browser keeps one.
  const kept = 'e2e-subscribed';
  const made = {
    endpoint,
    options: { applicationServerKey: null, userVisibleOnly: true },
    toJSON: () => ({ endpoint, keys: { p256dh: 'e2e-key', auth: 'e2e-auth' } }),
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
  test(`a thread offers notifications, asking nothing until Turn on, and Not now is remembered (${colorScheme})`, async ({
    page,
    browserName,
  }) => {
    test.skip(browserName === 'webkit', "Playwright's WebKit has no Push API to stand in for");
    await page.emulateMedia({ colorScheme });
    await page.addInitScript(standIns, ENDPOINT);
    const api = await fakeApi(page, { push: true });
    await page.goto(`/t/${THREAD}${SIGNED_IN}`);
    const banner = byTestId(page, 'push-banner');
    await expect(banner).toContainText(
      'Get a notification when Glave needs you, even with this tab closed.',
    );
    await expect(byTestId(page, 'thread-items')).toBeVisible();
    // The page asked the browser nothing as it opened.
    expect(await page.evaluate(() => (window as Asking).asked)).toBe(0);
    await expectAccessible(page);
    await byTestId(page, 'push-banner-not-now').click();
    await expect(banner).toBeHidden();
    await page.reload();
    await expect(byTestId(page, 'thread-items')).toBeVisible();
    await expect(banner).toBeHidden();
    expect(await page.evaluate(() => (window as Asking).asked)).toBe(0);
    expect(api.calls).not.toContain('push/subscribe');
  });
}

test('Turn on registers glave’s worker under its own policy, and the server keeps this browser', async ({
  page,
  browserName,
}) => {
  test.skip(browserName === 'webkit', "Playwright's WebKit has no Push API to stand in for");
  await page.addInitScript(standIns, ENDPOINT);
  const api = await fakeApi(page, { push: true });
  await page.goto(`/t/${THREAD}${SIGNED_IN}`);
  const banner = byTestId(page, 'push-banner');
  await expect(banner).toBeVisible();
  await byTestId(page, 'push-banner-on').click();
  await expect(banner).toBeHidden();
  expect(await page.evaluate(() => (window as Asking).asked)).toBe(1);
  const subscribed = (): readonly unknown[] =>
    api.requests.filter(({ method }) => method === 'push/subscribe').map(({ params }) => params);
  await expect.poll(subscribed).toEqual([
    {
      endpoint: ENDPOINT,
      p256dh: 'e2e-key',
      auth: 'e2e-auth',
      device: expect.stringContaining('Chrome'),
    },
  ]);
  // The worker the server's CSP and Trusted Types let the page register, running.
  const worker = (): Promise<string | null> =>
    page.evaluate(async () => {
      const registration = await navigator.serviceWorker.getRegistration('/');
      return registration?.active?.scriptURL ?? null;
    });
  await expect.poll(worker).toMatch(/\/push-worker\.js$/u);
  await page.goto(`/settings${SIGNED_IN}`);
  await expect(byTestId(page, 'notifications-state')).toHaveText(
    'This browser gets notifications.',
  );
  await expect(byTestId(page, 'notifications-device')).toContainText('This browser');
  await expectAccessible(page);
});
