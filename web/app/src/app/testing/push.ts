import type { EnvironmentProviders, Provider } from '@angular/core';
import { TestBed } from '@angular/core/testing';
import type { PushDevice, PushInfo } from '@glave/client';

import type { PushBrowser } from '../lib/push';
import { TOKEN_KEY } from '../lib/token';
import { PUSH_BROWSER } from '../services/push.service';
import { SessionService } from '../services/session.service';
import { FAKE_SERVER, type FakeSession, SIGNED_IN, provideFakeSession } from './fake-session';

/** The server's key, as `push/get` gives it. */
export const KEY = 'AQIDBA';

/** Where the fake push service takes this browser's pushes. */
export const ENDPOINT = 'https://fcm.googleapis.com/fcm/send/this-browser';

/** A browser the server pushes to, as it lists it. */
export function device(id: string, name: string, current = false): PushDevice {
  return { id, name, added: '2026-11-07T09:00:00.000Z', ...(current ? { current } : {}) };
}

/** What a server that pushes says. */
export function offered(devices: readonly PushDevice[] = []): PushInfo {
  return { kind: 'offered', key: KEY, devices };
}

/**
 * A browser that takes pushes, whose person has decided `permission` and answers `answer`
 * when asked: it records each time it asks, each worker registered, and each subscription made.
 */
export class FakePushBrowser {
  readonly asked: string[] = [];
  readonly registered: string[] = [];
  readonly made: PushSubscriptionOptionsInit[] = [];
  readonly unsubscribed: string[] = [];
  /** Called as the browser is unsubscribed, before it says so. */
  onUnsubscribe: (() => void) | null = null;
  private subscription: PushSubscription | null = null;
  readonly browser: PushBrowser;

  constructor(permission: NotificationPermission, answer: NotificationPermission = 'granted') {
    const pushManager = {
      getSubscription: () => Promise.resolve(this.subscription),
      subscribe: (options: PushSubscriptionOptionsInit) => {
        this.made.push(options);
        this.subscription = this.subscribed(options);
        return Promise.resolve(this.subscription);
      },
    };
    const registration = { pushManager } as unknown as ServiceWorkerRegistration;
    const workers = {
      register: (url: string) => {
        this.registered.push(url);
        return Promise.resolve(registration);
      },
      ready: Promise.resolve(registration),
      getRegistration: () => Promise.resolve(registration),
    } as unknown as ServiceWorkerContainer;
    const notifications = {
      permission,
      requestPermission: () => {
        this.asked.push('asked');
        return Promise.resolve(answer);
      },
    };
    this.browser = { notifications, workers };
  }

  /** It holds a subscription already, made with `key`. */
  holding(key: Uint8Array<ArrayBuffer>): void {
    this.subscription = this.subscribed({ applicationServerKey: key });
  }

  /** A subscription at `ENDPOINT`, made with `options`' key. */
  private subscribed(options: PushSubscriptionOptionsInit): PushSubscription {
    const key = options.applicationServerKey;
    return {
      endpoint: ENDPOINT,
      options: { applicationServerKey: key instanceof Uint8Array ? key.buffer : null },
      toJSON: () => ({ endpoint: ENDPOINT, keys: { p256dh: 'browser-key', auth: 'browser-auth' } }),
      unsubscribe: () => {
        this.onUnsubscribe?.();
        this.unsubscribed.push(ENDPOINT);
        this.subscription = null;
        return Promise.resolve(true);
      },
    } as unknown as PushSubscription;
  }
}

/**
 * Sets up a tab signed in to `fake`, a server that pushes unless `pushes` is false, in
 * `browser`, or one that takes no pushes when it is `null`, with `providers` beside.
 */
export async function pushingTab(
  fake: FakeSession,
  browser: PushBrowser | null,
  pushes = true,
  providers: readonly (Provider | EnvironmentProviders)[] = [],
): Promise<void> {
  fake.answer('server/discover', { id: FAKE_SERVER, push: pushes });
  TestBed.configureTestingModule({
    providers: [
      ...provideFakeSession(fake),
      { provide: PUSH_BROWSER, useValue: browser },
      ...providers,
    ],
  });
  localStorage.setItem(TOKEN_KEY, JSON.stringify(SIGNED_IN));
  await TestBed.inject(SessionService).start();
}
