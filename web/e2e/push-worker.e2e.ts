import { readFileSync } from 'node:fs';
import { runInNewContext } from 'node:vm';

import { expect, test } from './support';

/** The worker's own source, as glave serves it. */
const SOURCE = readFileSync(new URL('../app/public/push-worker.js', import.meta.url), 'utf8');

/** A thread a push names. */
const THREAD = '019a0000-0000-7000-8000-000000000001';

/** A window of the app the worker finds open. */
interface Window {
  readonly focused: number;
  readonly went: string[];
}

/**
 * The worker run in a scope of its own, as a browser runs it: the notifications it shows, the
 * windows it opens and moves, and `dispatch`, which hands it an event and waits for what it was
 * told to wait for.
 */
function started(windows: readonly { moves: boolean }[] = []): {
  readonly shown: { title: string; options: Record<string, unknown> }[];
  readonly opened: string[];
  readonly open: Window[];
  dispatch(type: string, event: Record<string, unknown>): Promise<void>;
} {
  const listeners = new Map<string, (event: unknown) => void>();
  const shown: { title: string; options: Record<string, unknown> }[] = [];
  const opened: string[] = [];
  const open = windows.map(() => ({ focused: 0, went: [] as string[] }));
  const clients = windows.map(({ moves }, at) => {
    const kept = open[at];
    const client = {
      focus: () => {
        if (kept !== undefined) {
          Object.assign(kept, { focused: kept.focused + 1 });
        }
        return Promise.resolve(client);
      },
      navigate: (url: string) => {
        kept?.went.push(url);
        return Promise.resolve(moves ? client : null);
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
  runInNewContext(SOURCE, { self, URL });
  return {
    shown,
    opened,
    open,
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

/** A tap on a notification that names `thread`. */
function tap(thread: unknown): Record<string, unknown> {
  return { notification: { data: { thread }, close: () => undefined } };
}

test('the worker shows what the server pushed, one notification a thread that alerts again', async () => {
  const worker = started();
  await worker.dispatch('push', push({ text: 'Needs you: fix-login', thread: THREAD }));
  expect(worker.shown).toEqual([
    {
      title: 'Needs you: fix-login',
      options: { tag: THREAD, renotify: true, data: { thread: THREAD }, icon: '/icon-192.png' },
    },
  ]);
});

test('a push the worker cannot read still shows, in the words the page gave it', async () => {
  const worker = started();
  await worker.dispatch('push', push(new SyntaxError('not JSON')));
  await worker.dispatch('push', push({ text: 'Done', thread: '../settings' }));
  expect(worker.shown.map(({ title, options }) => [title, options['tag']])).toEqual([
    ['Open glave', 'glave'],
    ['Done', 'glave'],
  ]);
});

test('a tapped notification opens its thread in the app’s open window, else a new one', async () => {
  const moving = started([{ moves: true }]);
  await moving.dispatch('notificationclick', tap(THREAD));
  expect(moving.open).toEqual([{ focused: 1, went: [`/t/${THREAD}`] }]);
  expect(moving.opened).toEqual([]);

  const none = started();
  await none.dispatch('notificationclick', tap(THREAD));
  expect(none.opened).toEqual([`/t/${THREAD}`]);

  // A window this browser won't move, and a notification that names no thread.
  const stuck = started([{ moves: false }]);
  await stuck.dispatch('notificationclick', tap('javascript:alert(1)'));
  expect(stuck.opened).toEqual(['/']);
});
