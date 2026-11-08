import { DOCUMENT } from '@angular/common';
import { InjectionToken, Service, computed, inject, signal } from '@angular/core';
import { ApiError, type PushDevice, type PushInfo } from '@glave/client';

import {
  type PushBrowser,
  deviceName,
  keyBytes,
  needsHomeScreen,
  pushBrowser,
  sameKey,
  subscribed,
} from '../lib/push';
import { readStored, storageOf, writeStored } from '../lib/storage';
import { sentence, unreached } from '../lib/text';
import { onChange } from './changes.service';
import { SessionService } from './session.service';

/** What this page's browser has to take pushes with, or `null` where it has none. */
export const PUSH_BROWSER = new InjectionToken<PushBrowser | null>('PUSH_BROWSER', {
  providedIn: 'root',
  factory: () => pushBrowser(inject(DOCUMENT).defaultView),
});

/** Where the service worker that shows glave's notifications is served, and the only one. */
export const WORKER = '/push-worker.js';

/** The Trusted Types policy that names the worker's script (the server's CSP allows it). */
const POLICY = 'glave-push';

/** Where "Not now" is remembered, in this browser alone. */
const NOT_NOW = 'glave.push.notNow';

/** How long the worker may take to start, after it is fetched: a script of a few lines. */
const READY_WITHIN = 10_000;

/** What a page's Trusted Types make policies with, where the browser has them. */
interface TrustedTypes {
  createPolicy(
    name: string,
    rules: { createScriptURL(url: string): string },
  ): { createScriptURL(url: string): unknown };
}

/** `promise`, or a failure once `within` milliseconds pass without it. */
async function limited<T>(promise: Promise<T>, within: number): Promise<T> {
  let timer: ReturnType<typeof setTimeout> | undefined;
  const late = new Promise<never>((_, reject) => {
    timer = setTimeout(() => {
      reject(new Error(`the notification worker did not start within ${String(within)} ms`));
    }, within);
  });
  try {
    return await Promise.race([promise, late]);
  } finally {
    clearTimeout(timer);
  }
}

/**
 * The person's push notifications (Web Push) in this browser: whether to offer them, turning
 * them on (the permission asked for only in the click that turns them on) and off, which kinds
 * the server pushes, and the browsers it pushes to, as the server keeps them. A notification
 * tapped opens its thread through the worker itself (`push-worker.js`).
 */
@Service()
export class PushService {
  private readonly session = inject(SessionService);
  private readonly document = inject(DOCUMENT);
  private readonly browser = inject(PUSH_BROWSER);
  private readonly storage = storageOf(this.document);
  private readonly permitted = signal<NotificationPermission | null>(
    this.browser?.notifications.permission ?? null,
  );
  private readonly putOff = signal(readStored(this.storage, NOT_NOW) === true);
  private readonly read = signal<PushInfo | null>(null);
  private readonly working = signal(false);
  private readonly failed = signal<string | null>(null);
  /** The device the server keeps for this browser, by its id, when it keeps one. */
  private readonly mine = signal<string | null>(null);
  private policy: ReturnType<TrustedTypes['createPolicy']> | null = null;

  /** Whether the server pushes notifications, as it said before the app started. */
  readonly pushes = this.session.push;
  /** Whether the server pushes notifications and this browser can show them. */
  readonly possible = computed(() => this.session.push() && this.browser !== null);
  /** Whether to offer them in the thread's banner: possible, and not decided in this browser. */
  readonly offered = computed(
    () => this.possible() && this.permitted() === 'default' && !this.putOff(),
  );
  /** Whether the person blocked them for this site in the browser's own settings. */
  readonly blocked = computed(() => this.permitted() === 'denied');
  /** Whether this is an iPhone's or iPad's Safari, which pushes only to the Home Screen's app. */
  readonly homeScreenFirst = computed(
    () => this.session.push() && this.browser === null && this.outsideHomeScreen(),
  );
  /** What the server said, `null` until it has. */
  readonly info = this.read.asReadonly();
  /** The browsers the server pushes to, this one marked by `current`. */
  readonly devices = computed<readonly PushDevice[]>(() => {
    const info = this.read();
    const mine = this.mine();
    return info?.kind === 'offered'
      ? info.devices.map((device) => ({ ...device, current: device.id === mine }))
      : [];
  });
  /** Whether the server pushes to this browser. */
  readonly on = computed(() => this.devices().some((device) => device.current === true));
  /** Whether a change is under way. */
  readonly busy = this.working.asReadonly();
  /** Why the last change did not happen, until the next. */
  readonly problem = this.failed.asReadonly();

  constructor() {
    onChange('push', () => void this.load());
  }

  /** Puts the offer off in this browser, for good: Settings still turns them on. */
  notNow(): void {
    writeStored(this.storage, NOT_NOW, true);
    this.putOff.set(true);
  }

  /** Reads what the server says, this browser's own device marked. */
  async load(): Promise<void> {
    if (!this.session.push()) {
      return;
    }
    await this.change(async () => {
      const endpoint = (await this.subscription())?.endpoint ?? null;
      return this.session.call('push/get', endpoint === null ? {} : { endpoint });
    });
  }

  /**
   * Turns notifications on in this browser, from the person's click: the browser asks them
   * first, before anything else is awaited, as Safari asks only in a click; then this browser is
   * subscribed with the server's key, and the server keeps it.
   */
  async turnOn(): Promise<void> {
    const browser = this.browser;
    if (browser === null) {
      return;
    }
    const asked = browser.notifications.requestPermission();
    await this.change(async () => {
      const permission = await asked;
      this.permitted.set(permission);
      if (permission !== 'granted') {
        return null;
      }
      const offered = await this.session.call('push/get', {});
      if (offered.push.kind !== 'offered') {
        return offered;
      }
      const key = keyBytes(offered.push.key);
      const registration = await this.register(browser.workers);
      let existing = await registration.pushManager.getSubscription();
      if (existing !== null && !sameKey(existing.options.applicationServerKey, key)) {
        // Made with another key, which the server no longer signs with.
        await existing.unsubscribe();
        existing = null;
      }
      const subscription =
        existing ??
        (await registration.pushManager.subscribe({
          userVisibleOnly: true,
          applicationServerKey: key,
        }));
      const kept = subscribed(subscription.toJSON());
      if (kept === null) {
        throw new Error('the browser gave a subscription without its keys');
      }
      const device = deviceName(this.document.defaultView?.navigator.userAgent ?? '');
      return this.session.call('push/subscribe', { ...kept, device });
    });
  }

  /** Turns them off in this browser: it unsubscribes, and the server forgets it. */
  async turnOff(): Promise<void> {
    const device = this.mine();
    await this.change(async () => {
      await (await this.subscription())?.unsubscribe();
      return device === null ? null : this.session.call('push/remove', { device });
    });
  }

  /** Forgets one of the person's browsers, which is told nothing more. */
  async remove(device: string): Promise<void> {
    await this.change(() => this.session.call('push/remove', { device }));
  }

  /** Chooses which kinds are pushed. */
  async choose(waiting: boolean, finished: boolean): Promise<void> {
    await this.change(() => this.session.call('push/set', { waiting, finished }));
  }

  /** Runs `asked`, keeping what the server answers and why it failed, one change at a time. */
  private async change(asked: () => Promise<{ readonly push: PushInfo } | null>): Promise<void> {
    this.working.set(true);
    this.failed.set(null);
    try {
      const answered = await asked();
      if (answered !== null) {
        this.keep(answered.push);
      }
    } catch (error: unknown) {
      this.failed.set(error instanceof ApiError ? sentence(error.message) : this.couldNot(error));
    } finally {
      this.working.set(false);
    }
  }

  /** Keeps what the server said, remembering this browser's device where it marks one. */
  private keep(info: PushInfo): void {
    if (info.kind === 'offered') {
      const marked = info.devices.find((device) => device.current === true);
      if (marked !== undefined) {
        this.mine.set(marked.id);
      } else if (!info.devices.some((device) => device.id === this.mine())) {
        this.mine.set(null);
      }
    }
    this.read.set(info);
  }

  /** What a failure the server did not explain is told as. */
  private couldNot(error: unknown): string {
    return error instanceof TypeError && error.message.includes('fetch')
      ? unreached()
      : $localize`:@@push.couldNot:This browser could not turn notifications on. Try again, or try another browser.`;
  }

  /** This browser's subscription, when its worker has one. */
  private async subscription(): Promise<PushSubscription | null> {
    const registration = await this.browser?.workers.getRegistration('/');
    return (await registration?.pushManager.getSubscription()) ?? null;
  }

  /** The worker registered, once it is ready to take pushes. */
  private async register(workers: ServiceWorkerContainer): Promise<ServiceWorkerRegistration> {
    await workers.register(this.workerUrl(), { scope: '/' });
    return limited(workers.ready, READY_WITHIN);
  }

  /**
   * The worker's address, as a page under Trusted Types must give it: through the one policy
   * that names it, made the first time.
   */
  private workerUrl(): string {
    const types = (this.document.defaultView as { trustedTypes?: TrustedTypes } | null)
      ?.trustedTypes;
    if (types === undefined) {
      return WORKER;
    }
    this.policy ??= types.createPolicy(POLICY, {
      createScriptURL: (url) => (url === WORKER ? url : ''),
    });
    // A TrustedScriptURL, which `register` takes in place of the text.
    return this.policy.createScriptURL(WORKER) as string;
  }

  /** Whether this browser is an iPhone's or iPad's outside the Home Screen. */
  private outsideHomeScreen(): boolean {
    const view = this.document.defaultView;
    if (view === null) {
      return false;
    }
    const standalone = view.matchMedia('(display-mode: standalone)').matches;
    return needsHomeScreen(view.navigator.userAgent, standalone);
  }
}
