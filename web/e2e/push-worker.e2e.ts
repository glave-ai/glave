import { readFileSync } from 'node:fs';
import { MessageChannel, type MessagePort } from 'node:worker_threads';
import { runInNewContext } from 'node:vm';

import { expect, test } from './support';

/** The worker's own source, as glave serves it. */
const SOURCE = readFileSync(new URL('../app/public/push-worker.js', import.meta.url), 'utf8');

/** A thread a push names, and its organization. */
const THREAD = '019a0000-0000-7000-8000-000000000001';
const ORG = '019a0000-0000-7000-8000-0000000000aa';

/** A window of the app the worker finds open, and what it did with it. */
interface Window {
  readonly visibilityState: 'visible' | 'hidden';
  /** Whether the app in it opens what it is asked to itself, saying so. */
  readonly answers: boolean;
  /** Whether the browser lets the worker load another page in it. */
  readonly moves: boolean;
}

/** What became of a window: how often it was brought forward, asked and loaded. */
interface Seen {
  focused: number;
  readonly asked: unknown[];
  readonly loaded: string[];
}

/**
 * The worker run in a scope of its own, as a browser runs it: the notifications it shows, the
 * windows it opens and moves, what it subscribes again, and `dispatch`, which hands it an event
 * and waits for what it was told to wait for.
 */
function started(windows: readonly Window[] = []): {
  readonly shown: { title: string; options: Record<string, unknown> }[];
  readonly opened: string[];
  readonly seen: Seen[];
  readonly subscribed: unknown[];
  dispatch(type: string, event: Record<string, unknown>): Promise<void>;
} {
  const listeners = new Map<string, (event: unknown) => void>();
  const shown: { title: string; options: Record<string, unknown> }[] = [];
  const opened: string[] = [];
  const subscribed: unknown[] = [];
  const seen = windows.map(() => ({ focused: 0, asked: [] as unknown[], loaded: [] as string[] }));
  const clients = windows.map((window, at) => {
    const kept = seen[at] ?? { focused: 0, asked: [], loaded: [] };
    const client = {
      visibilityState: window.visibilityState,
      focus: () => {
        kept.focused += 1;
        return Promise.resolve(client);
      },
      postMessage: (message: unknown, [port]: MessagePort[]) => {
        kept.asked.push(message);
        // The worker closes the channel once it is answered, or gives up waiting.
        if (window.answers) {
          port?.postMessage('opened');
        }
      },
      navigate: (url: string) => {
        kept.loaded.push(url);
        return Promise.resolve(window.moves ? client : null);
      },
    };
    return client;
  });
  const self = {
    location: { href: 'https://glave.example.com/push-worker.js?fallback=Open%20glave' },
    addEventListener: (type: string, listener: (event: unknown) => void) => {
      listeners.set(type, listener);
    },
    skipWaiting: () => Promise.resolve(),
    registration: {
      showNotification: (title: string, options: Record<string, unknown>) => {
        shown.push({ title, options });
        return Promise.resolve();
      },
      pushManager: {
        subscribe: (options: unknown) => {
          subscribed.push(options);
          return Promise.resolve({});
        },
      },
    },
    clients: {
      claim: () => Promise.resolve(),
      matchAll: () => Promise.resolve(clients),
      openWindow: (url: string) => {
        opened.push(url);
        return Promise.resolve(null);
      },
    },
  };
  runInNewContext(SOURCE, { self, URL, MessageChannel, setTimeout, clearTimeout, Date, String });
  return {
    shown,
    opened,
    seen,
    subscribed,
    async dispatch(type, event) {
      let waited: Promise<unknown> = Promise.resolve();
      listeners.get(type)?.({
        ...event,
        waitUntil: (promise: Promise<unknown>) => {
          waited = promise;
        },
      });
      await waited;
    },
  };
}

/** A push carrying `json`, as the browser hands it to the worker. */
function push(json: unknown): Record<string, unknown> {
  return {
    data: {
      json: () => {
        if (json instanceof Error) {
          throw json;
        }
        return json;
      },
    },
  };
}

/** A tap on a notification that names `thread` in `org`. */
function tap(thread: unknown, org: unknown = ORG): Record<string, unknown> {
  return { notification: { data: { thread, org }, close: () => undefined } };
}

test('what waits replaces its thread’s notification, alerting again; a task’s end alerts as one of its own', async () => {
  const worker = started();
  const waits = { text: 'Needs you: fix-login', thread: THREAD, org: ORG, kind: 'waiting' };
  await worker.dispatch('push', push(waits));
  await worker.dispatch('push', push({ ...waits, text: 'Done: fix-login', kind: 'ended' }));
  const [waiting, ended] = worker.shown;
  expect(waiting).toEqual({
    title: 'Needs you: fix-login',
    options: {
      tag: THREAD,
      renotify: true,
      data: { thread: THREAD, org: ORG },
      icon: '/icon-192.png',
    },
  });
  expect(ended?.title).toBe('Done: fix-login');
  expect(ended?.options['tag']).toMatch(new RegExp(`^${THREAD}:\\d+$`, 'u'));
  expect(ended?.options['renotify']).toBe(false);
});

test('a push the worker cannot read still shows, in the words the page gave it', async () => {
  const worker = started();
  await worker.dispatch('push', push(new SyntaxError('not JSON')));
  await worker.dispatch('push', push({ text: 'Done', thread: '../settings', org: 'x' }));
  expect(
    worker.shown.map(({ title, options }) => [title, options['tag'], options['data']]),
  ).toEqual([
    ['Open glave', 'glave', { thread: null, org: null }],
    ['Done', 'glave', { thread: null, org: null }],
  ]);
});

test('a tapped notification opens its thread in the window the person sees, which moves there itself', async () => {
  const worker = started([
    { visibilityState: 'hidden', answers: true, moves: true },
    { visibilityState: 'visible', answers: true, moves: true },
  ]);
  await worker.dispatch('notificationclick', tap(THREAD));
  const page = `/t/${THREAD}`;
  expect(worker.seen).toEqual([
    { focused: 0, asked: [], loaded: [] },
    { focused: 1, asked: [{ glave: 'open', page, org: ORG }], loaded: [] },
  ]);
  expect(worker.opened).toEqual([]);
});

test('an app that does not open it itself is loaded there in its organization, else a new window opens', async () => {
  const loading = started([{ visibilityState: 'visible', answers: false, moves: true }]);
  await loading.dispatch('notificationclick', tap(THREAD));
  expect(loading.seen[0]?.loaded).toEqual([`/t/${THREAD}#org=${ORG}`]);
  expect(loading.opened).toEqual([]);

  const none = started();
  await none.dispatch('notificationclick', tap(THREAD));
  expect(none.opened).toEqual([`/t/${THREAD}#org=${ORG}`]);

  // A window this browser won't move, and a notification that names no thread.
  const stuck = started([{ visibilityState: 'visible', answers: false, moves: false }]);
  await stuck.dispatch('notificationclick', tap('javascript:alert(1)', null));
  expect(stuck.opened).toEqual(['/']);
});

test('a subscription the browser renews is subscribed again with the same key', async () => {
  const worker = started();
  const key = new Uint8Array([1, 2, 3]).buffer;
  await worker.dispatch('pushsubscriptionchange', {
    oldSubscription: { options: { applicationServerKey: key } },
  });
  expect(worker.subscribed).toEqual([{ userVisibleOnly: true, applicationServerKey: key }]);
  // One with no key to subscribe again with is left to the app.
  await worker.dispatch('pushsubscriptionchange', { oldSubscription: null });
  expect(worker.subscribed).toHaveLength(1);
});
