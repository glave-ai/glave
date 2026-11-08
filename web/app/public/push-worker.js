// @ts-check
/**
 * glave's service worker, for push notifications alone (`specs/session.md`, "Push
 * notifications"). It shows what the server pushed, the server's own sentence: what waits for
 * the person replaces their thread's last notification, alerting again where the browser can
 * (`renotify`), and a task's end always alerts, under a tag of its own, as Safari, which has no
 * `renotify`, needs. Tapped, it opens that thread in its organization: in the window of the app
 * the person sees, which moves there itself without loading again, else a new one, which on an
 * iPhone is the app on the Home Screen. When the browser renews its subscription, it subscribes
 * again with the same key; the app tells the server of it when it next opens. It caches nothing
 * and handles no request, so the app loads as it does without it. The page names it with what
 * to say of a push it cannot read, in the person's language (`?fallback=`). Type-checked as the
 * app is (`app/tsconfig.worker.json`).
 */

const worker = /** @type {ServiceWorkerGlobalScope} */ (/** @type {unknown} */ (self));

/** An id as the server writes a thread's or an organization's. */
const ID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/u;

/** What a push this worker cannot read says, as the page gave it, else the app's name. */
const FALLBACK = new URL(worker.location.href).searchParams.get('fallback') ?? 'glave';

/** How long an open window of the app may take to say it opens the thread itself. */
const ANSWER_WITHIN = 1_000;

/**
 * `value` when it is an id, else `null`.
 *
 * @param {unknown} value
 * @returns {string | null}
 */
function id(value) {
  return typeof value === 'string' && ID.test(value) ? value : null;
}

/**
 * What a push says: the server's sentence, the thread and organization it is about when it
 * names them, and whether it is a task's end.
 *
 * @param {PushMessageData | null} data
 * @returns {{ text: string, thread: string | null, org: string | null, ended: boolean }}
 */
function said(data) {
  /** @type {unknown} */
  let read = null;
  try {
    read = data?.json() ?? null;
  } catch {
    // Not what the server sends: shown as the page said to.
  }
  const { text, thread, org, kind } =
    /** @type {{ text?: unknown, thread?: unknown, org?: unknown, kind?: unknown }} */ (read ?? {});
  return {
    text: typeof text === 'string' && text !== '' ? text : FALLBACK,
    thread: id(thread),
    org: id(org),
    ended: kind === 'ended',
  };
}

/**
 * Whether the app in `client` took `page` to open itself, saying so within
 * [`ANSWER_WITHIN`] ms; an app that does not know `org` as the one it works in lets it be.
 *
 * @param {WindowClient} client
 * @param {string} page
 * @param {string | null} org
 * @returns {Promise<boolean>}
 */
function opened(client, page, org) {
  return new Promise((resolve) => {
    const channel = new MessageChannel();
    const late = setTimeout(() => {
      channel.port1.close();
      resolve(false);
    }, ANSWER_WITHIN);
    channel.port1.onmessage = () => {
      clearTimeout(late);
      channel.port1.close();
      resolve(true);
    };
    client.postMessage({ glave: 'open', page, org }, [channel.port2]);
  });
}

/**
 * Opens `thread`'s page in `org`: in the window of the app the person sees, brought forward, by
 * the app itself or else loaded there, or in a new window.
 *
 * @param {string | null} thread
 * @param {string | null} org
 */
async function open(thread, org) {
  const page = thread === null ? '/' : `/t/${thread}`;
  const loaded = org === null ? page : `${page}#org=${org}`;
  const windows = await worker.clients.matchAll({ type: 'window', includeUncontrolled: true });
  const app = windows.find((each) => each.visibilityState === 'visible') ?? windows[0];
  if (app !== undefined) {
    try {
      const focused = await app.focus();
      if ((await opened(focused, page, org)) || (await focused.navigate(loaded)) !== null) {
        return;
      }
    } catch {
      // A window this browser won't bring forward or move: the thread opens in a new one.
    }
  }
  await worker.clients.openWindow(loaded);
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
  const { text, thread, org, ended } = said(event.data);
  const tag = thread ?? 'glave';
  /** @type {NotificationOptions & { renotify: boolean }} */
  const options = {
    // What waits replaces the thread's last, still alerting; a task's end is one of its own.
    tag: ended ? `${tag}:${String(Date.now())}` : tag,
    renotify: !ended,
    data: { thread, org },
    icon: '/icon-192.png',
  };
  event.waitUntil(worker.registration.showNotification(text, options));
});

worker.addEventListener('notificationclick', (event) => {
  event.notification.close();
  const { thread, org } = /** @type {{ thread?: unknown, org?: unknown }} */ (
    event.notification.data ?? {}
  );
  event.waitUntil(open(id(thread), id(org)));
});

// The browser renewed the subscription: subscribed again with the same key, the new one reaches
// the server when the app next opens, as it finds the server without it.
worker.addEventListener('pushsubscriptionchange', (event) => {
  const change = /** @type {ExtendableEvent & { oldSubscription?: PushSubscription | null }} */ (
    /** @type {unknown} */ (event)
  );
  const key = change.oldSubscription?.options.applicationServerKey ?? null;
  if (key === null) {
    return;
  }
  const again = worker.registration.pushManager.subscribe({
    userVisibleOnly: true,
    applicationServerKey: key,
  });
  change.waitUntil(
    again.then(
      () => undefined,
      () => undefined,
    ),
  );
});
