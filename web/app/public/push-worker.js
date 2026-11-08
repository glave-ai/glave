// @ts-check
/**
 * glave's service worker, for push notifications alone (`specs/session.md`, "Push
 * notifications"). It shows what the server pushed, the server's own sentence, one notification
 * a thread, alerting again when a thread's replaces its last, and opens that thread when the
 * person taps it: in the app's window when one is open, else a new one, which on an iPhone is
 * the app on the Home Screen. It caches nothing and handles no request, so the app loads as it
 * does without it. The page names it with what to say of a push it cannot read, in the person's
 * language (`?fallback=`). Type-checked as the app is (`app/tsconfig.worker.json`).
 */

const worker = /** @type {ServiceWorkerGlobalScope} */ (/** @type {unknown} */ (self));

/** A thread's id, as the server names the thread a push is about. */
const THREAD = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/u;

/** What a push this worker cannot read says, as the page gave it, else the app's name. */
const FALLBACK = new URL(worker.location.href).searchParams.get('fallback') ?? 'glave';

/**
 * What a push says: the server's sentence, and the thread it is about when it names one.
 *
 * @param {PushMessageData | null} data
 * @returns {{ text: string, thread: string | null }}
 */
function said(data) {
  /** @type {unknown} */
  let read = null;
  try {
    read = data?.json() ?? null;
  } catch {
    // Not what the server sends: shown as the page said to.
  }
  const { text, thread } = /** @type {{ text?: unknown, thread?: unknown }} */ (read ?? {});
  return {
    text: typeof text === 'string' && text !== '' ? text : FALLBACK,
    thread: typeof thread === 'string' && THREAD.test(thread) ? thread : null,
  };
}

/**
 * Opens `thread`'s page: in a window of the app that is open, brought forward, else in a new
 * one.
 *
 * @param {string | null} thread
 */
async function open(thread) {
  const page = thread === null ? '/' : `/t/${thread}`;
  const windows = await worker.clients.matchAll({ type: 'window' });
  const app = windows[0];
  if (app !== undefined) {
    try {
      const focused = await app.focus();
      if ((await focused.navigate(page)) !== null) {
        return;
      }
    } catch {
      // A window this browser won't bring forward or move: the thread opens in a new one.
    }
  }
  await worker.clients.openWindow(page);
}

// A new version takes over at once, and the app's open pages with it, so a tapped notification
// opens its thread in one: it keeps nothing a page could depend on, and handles no request.
worker.addEventListener('install', () => {
  void worker.skipWaiting();
});

worker.addEventListener('activate', (event) => {
  event.waitUntil(worker.clients.claim());
});

worker.addEventListener('push', (event) => {
  const { text, thread } = said(event.data);
  /** @type {NotificationOptions & { renotify: boolean }} */
  const options = {
    tag: thread ?? 'glave',
    // A thread's next notification replaces its last, and still alerts.
    renotify: true,
    data: { thread },
    icon: '/icon-192.png',
  };
  event.waitUntil(worker.registration.showNotification(text, options));
});

worker.addEventListener('notificationclick', (event) => {
  event.notification.close();
  const { thread } = /** @type {{ thread?: unknown }} */ (event.notification.data ?? {});
  event.waitUntil(open(typeof thread === 'string' && THREAD.test(thread) ? thread : null));
});
