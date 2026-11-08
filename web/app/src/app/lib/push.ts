/**
 * What a browser needs to take glave's push notifications (Web Push), as pure functions: whether
 * it can at all, the key a subscription is made with, what a subscription sends the server, and
 * what the person knows the browser as.
 */

/** The parts of a browser that push needs: notifications' permission, and service workers. */
export interface PushBrowser {
  readonly notifications: {
    readonly permission: NotificationPermission;
    requestPermission(): Promise<NotificationPermission>;
  };
  readonly workers: ServiceWorkerContainer;
}

/**
 * What `view` has to take pushes with, or `null` where it has none: a page that is no secure
 * context, an old browser, or Safari on an iPhone or iPad outside the Home Screen.
 */
export function pushBrowser(view: Window | null): PushBrowser | null {
  if (view === null) {
    return null;
  }
  const { Notification: notifications } = view as Window & {
    readonly Notification?: PushBrowser['notifications'];
  };
  if (
    notifications === undefined ||
    !('PushManager' in view) ||
    !('serviceWorker' in view.navigator)
  ) {
    return null;
  }
  return { notifications, workers: view.navigator.serviceWorker };
}

/** What a browser says of itself: its user agent, and how many touches its screen takes. */
export interface Described {
  readonly userAgent: string;
  readonly maxTouchPoints: number;
}

/**
 * Whether `browser` is an iPad that says it is a Mac, as iPadOS's Safari does by default: a
 * Mac's user agent on a touch screen, which no Mac has.
 */
function iPadAsMac({ userAgent, maxTouchPoints }: Described): boolean {
  return /\bMacintosh\b/u.test(userAgent) && maxTouchPoints > 1;
}

/**
 * Whether `browser` is an iPhone's or an iPad's outside the Home Screen (`standalone` says
 * whether the app was opened from there), where Safari gives a page no pushes until it is added.
 */
export function needsHomeScreen(browser: Described, standalone: boolean): boolean {
  return (/\b(iPhone|iPad)\b/u.test(browser.userAgent) || iPadAsMac(browser)) && !standalone;
}

/** The server's key as `PushManager.subscribe` takes it: base64url made bytes. */
export function keyBytes(key: string): Uint8Array<ArrayBuffer> {
  const base64 = key.replaceAll('-', '+').replaceAll('_', '/');
  const padded = base64.padEnd(base64.length + ((4 - (base64.length % 4)) % 4), '=');
  return Uint8Array.from(atob(padded), (char) => char.charCodeAt(0));
}

/** Whether `subscribed`, a subscription's key, is the server's `key`. */
export function sameKey(subscribed: ArrayBuffer | null, key: Uint8Array): boolean {
  if (subscribed === null) {
    return false;
  }
  const bytes = new Uint8Array(subscribed);
  return bytes.length === key.length && bytes.every((byte, at) => byte === key[at]);
}

/** A subscription as the server keeps it: where its push service takes pushes, and its keys. */
export interface Subscribed {
  readonly endpoint: string;
  readonly p256dh: string;
  readonly auth: string;
}

/** What `subscription` (`PushSubscription.toJSON`) gives the server; `null` without its keys. */
export function subscribed(subscription: PushSubscriptionJSON): Subscribed | null {
  const { endpoint, keys } = subscription;
  const p256dh = keys?.['p256dh'];
  const auth = keys?.['auth'];
  if (endpoint === undefined || p256dh === undefined || auth === undefined) {
    return null;
  }
  return { endpoint, p256dh, auth };
}

/** Browsers by what their user agent says, the more particular first: Edge says Chrome too. */
const BROWSERS: readonly (readonly [RegExp, string])[] = [
  [/\bEdg(e|A|iOS)?\//u, 'Edge'],
  [/\b(Firefox|FxiOS)\//u, 'Firefox'],
  [/\bSamsungBrowser\//u, 'Samsung Internet'],
  [/\bOPR\//u, 'Opera'],
  [/\b(Chrome|CriOS)\//u, 'Chrome'],
  [/\bSafari\//u, 'Safari'],
];

/** Systems by what a user agent says, the more particular first: an iPhone says Mac OS X too. */
const SYSTEMS: readonly (readonly [RegExp, string])[] = [
  [/\biPhone\b/u, 'iPhone'],
  [/\biPad\b/u, 'iPad'],
  [/\bAndroid\b/u, 'Android'],
  [/\bCrOS\b/u, 'ChromeOS'],
  [/\bMac OS X\b/u, 'Mac'],
  [/\bWindows\b/u, 'Windows'],
  [/\bLinux\b/u, 'Linux'],
];

/** The first of `names` whose pattern `userAgent` matches. */
function named(names: readonly (readonly [RegExp, string])[], userAgent: string): string | null {
  return names.find(([pattern]) => pattern.test(userAgent))?.[1] ?? null;
}

/**
 * What the person knows `browser` as: "Safari on Mac", "Safari on iPad" for an iPad that says
 * it is a Mac, "iPhone" for an app on the Home Screen, which names no browser, or plain words
 * where it names neither.
 */
export function deviceName(browser: Described): string {
  const { userAgent } = browser;
  const name = named(BROWSERS, userAgent);
  const system = iPadAsMac(browser) ? 'iPad' : named(SYSTEMS, userAgent);
  if (name !== null && system !== null) {
    return $localize`:@@push.device:${name}:BROWSER: on ${system}:SYSTEM:`;
  }
  return name ?? system ?? $localize`:@@push.deviceUnknown:A browser`;
}
