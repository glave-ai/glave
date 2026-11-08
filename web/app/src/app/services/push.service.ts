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
import { readSession } from '../lib/token';
import { onChange } from './changes.service';
import { OrganizationsService } from './organizations.service';
import { SESSION_CLIENT_FACTORY, SessionService } from './session.service';
import { TapsService } from './taps.service';

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

/**
 * Where it is remembered that the person turned notifications on in this browser, so a
 * subscription the server no longer has is given it again, and one turned off is not.
 */
const ON_HERE = 'glave.push.on';

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

/** The device `info` marks as the browser that asked, if it marks one. */
function marked(info: PushInfo): PushDevice | undefined {
  return info.kind === 'offered'
    ? info.devices.find((device) => device.current === true)
    : undefined;
}

/**
 * The person's push notifications (Web Push) in this browser: whether to offer them, turning
 * them on (the permission asked for only in the click that turns them on) and off, the browsers
 * the server pushes to, as it keeps them, and this one given to the server again, quietly, when
 * the person turned them on here and the server no longer has it. A notification tapped opens
 * its thread through the worker itself (`push-worker.js`).
 */
@Service()
export class PushService {
  private readonly session = inject(SessionService);
  private readonly organizations = inject(OrganizationsService);
  /** What calls the server in another of the person's organizations. */
  private readonly clientIn = inject(SESSION_CLIENT_FACTORY);
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
    // A tapped notification opens here from now on, when the worker asks.
    inject(TapsService);
    void this.resync();
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
      const kept = await this.subscribeHere(browser);
      writeStored(this.storage, ON_HERE, true);
      return kept;
    });
  }

  /**
   * Turns them off in this browser: it unsubscribes, and every organization of the person's
   * the server pushed it for forgets it.
   */
  async turnOff(): Promise<void> {
    await this.change(async () => {
      writeStored(this.storage, ON_HERE, false);
      const subscription = await this.subscription();
      await subscription?.unsubscribe();
      if (subscription === null) {
        const device = this.mine();
        return device === null ? null : this.session.call('push/remove', { device });
      }
      await this.forgetElsewhere(subscription.endpoint);
      return this.forgetHere(subscription.endpoint);
    });
  }

  /**
   * Stops this browser's notifications as the person signs out of it, so no one who uses it
   * next sees theirs: it unsubscribes first, then the server forgets it, unless it can't be
   * reached, when it forgets it as its session ends. Signing out goes on whatever comes of it.
   */
  async signingOut(): Promise<void> {
    writeStored(this.storage, ON_HERE, false);
    let subscription: PushSubscription | null = null;
    try {
      subscription = await this.subscription();
      await subscription?.unsubscribe();
    } catch {
      // A browser whose worker can't be reached has nothing here to stop.
    }
    if (subscription === null || !this.session.push()) {
      return;
    }
    try {
      await this.forgetHere(subscription.endpoint);
    } catch {
      // The server forgets it once the session it was subscribed in ends, as it does now.
    }
  }

  /** Forgets one of the person's browsers, which is told nothing more. */
  async remove(device: string): Promise<void> {
    await this.change(() => this.session.call('push/remove', { device }));
  }

  /**
   * Gives the server this browser again, quietly, when the person turned notifications on here,
   * the browser still lets them, and the server no longer has it: its subscription renewed by
   * the browser, or forgotten once its push service failed.
   */
  private async resync(): Promise<void> {
    const browser = this.browser;
    const on = readStored(this.storage, ON_HERE) === true;
    if (!on || browser === null || !this.possible() || this.permitted() !== 'granted') {
      return;
    }
    try {
      const endpoint = (await this.subscription())?.endpoint ?? null;
      const asked = await this.session.call('push/get', endpoint === null ? {} : { endpoint });
      this.keep(asked.push);
      if (asked.push.kind === 'offered' && marked(asked.push) === undefined) {
        this.keep((await this.subscribeHere(browser)).push);
      }
    } catch {
      // Tried again when the app next opens.
    }
  }

  /** Subscribes this browser with the server's key, and has the server keep it. */
  private async subscribeHere(browser: PushBrowser): Promise<{ readonly push: PushInfo }> {
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
    const device = deviceName(this.navigator());
    return this.session.call('push/subscribe', { ...kept, device });
  }

  /** Has the server forget the browser at `endpoint` in the organization this tab works in. */
  private async forgetHere(endpoint: string): Promise<{ readonly push: PushInfo }> {
    const asked = await this.session.call('push/get', { endpoint });
    const here = marked(asked.push);
    return here === undefined ? asked : this.session.call('push/remove', { device: here.id });
  }

  /** Has the server forget the browser at `endpoint` in the person's other organizations. */
  private async forgetElsewhere(endpoint: string): Promise<void> {
    await this.organizations.load();
    const others = this.organizations.organizations().filter((each) => each.current !== true);
    for (const { id } of others) {
      // This browser's session, working in `id`: what this tab works in stays as it is.
      const there = this.clientIn(
        () => readSession(this.storage)?.token ?? '',
        () => id,
      );
      try {
        const asked = await there.call('push/get', { endpoint });
        const device = marked(asked.push);
        if (device !== undefined) {
          await there.call('push/remove', { device: device.id });
        }
      } catch {
        // One that can't be reached forgets it once its push service says it is gone.
      }
    }
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
      const here = marked(info);
      if (here !== undefined) {
        this.mine.set(here.id);
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
   * The worker's address, telling it what to show of a push it cannot read, in the person's
   * language; as a page under Trusted Types must give it, through the one policy that names it,
   * made the first time.
   */
  private workerUrl(): string {
    const fallback = $localize`:@@push.fallback:glave needs you`;
    const url = `${WORKER}?fallback=${encodeURIComponent(fallback)}`;
    const types = (this.document.defaultView as { trustedTypes?: TrustedTypes } | null)
      ?.trustedTypes;
    if (types === undefined) {
      return url;
    }
    this.policy ??= types.createPolicy(POLICY, {
      createScriptURL: (asked) => (asked.startsWith(`${WORKER}?fallback=`) ? asked : ''),
    });
    // A TrustedScriptURL, which `register` takes in place of the text.
    return this.policy.createScriptURL(url) as string;
  }

  /** What this browser says of itself. */
  private navigator(): { readonly userAgent: string; readonly maxTouchPoints: number } {
    const navigator = this.document.defaultView?.navigator;
    return {
      userAgent: navigator?.userAgent ?? '',
      maxTouchPoints: navigator?.maxTouchPoints ?? 0,
    };
  }

  /** Whether this browser is an iPhone's or iPad's outside the Home Screen. */
  private outsideHomeScreen(): boolean {
    const view = this.document.defaultView;
    if (view === null) {
      return false;
    }
    const standalone = view.matchMedia('(display-mode: standalone)').matches;
    return needsHomeScreen(this.navigator(), standalone);
  }
}
