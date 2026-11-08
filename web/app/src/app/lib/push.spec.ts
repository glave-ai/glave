import { deviceName, keyBytes, needsHomeScreen, pushBrowser, sameKey, subscribed } from './push';

const SAFARI_MAC =
  'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/26.0 Safari/605.1.15';
const IPHONE_APP =
  'Mozilla/5.0 (iPhone; CPU iPhone OS 26_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Mobile/15E148';
const IPHONE_SAFARI =
  'Mozilla/5.0 (iPhone; CPU iPhone OS 26_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/26.0 Mobile/15E148 Safari/604.1';
const CHROME_WINDOWS =
  'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/142.0.0.0 Safari/537.36';
const EDGE =
  'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/142.0.0.0 Safari/537.36 Edg/142.0.0.0';
const FIREFOX_LINUX = 'Mozilla/5.0 (X11; Linux x86_64; rv:145.0) Gecko/20100101 Firefox/145.0';
const CHROME_ANDROID =
  'Mozilla/5.0 (Linux; Android 16; Pixel 9) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/142.0.0.0 Mobile Safari/537.36';

/** A browser saying `userAgent`, on a screen that takes `maxTouchPoints` touches. */
function said(
  userAgent: string,
  maxTouchPoints = 0,
): { userAgent: string; maxTouchPoints: number } {
  return { userAgent, maxTouchPoints };
}

describe('deviceName', () => {
  it('names the browser and the system it runs on, the more particular first', () => {
    expect(deviceName(said(SAFARI_MAC))).toBe('Safari on Mac');
    expect(deviceName(said(IPHONE_SAFARI, 5))).toBe('Safari on iPhone');
    expect(deviceName(said(CHROME_WINDOWS))).toBe('Chrome on Windows');
    expect(deviceName(said(EDGE))).toBe('Edge on Windows');
    expect(deviceName(said(FIREFOX_LINUX))).toBe('Firefox on Linux');
    expect(deviceName(said(CHROME_ANDROID, 5))).toBe('Chrome on Android');
  });

  it('names an iPad that says it is a Mac by its touch screen', () => {
    expect(deviceName(said(SAFARI_MAC, 5))).toBe('Safari on iPad');
  });

  it('names an app on the Home Screen by its device, and an unknown browser in plain words', () => {
    expect(deviceName(said(IPHONE_APP, 5))).toBe('iPhone');
    expect(deviceName(said('curl/8.0'))).toBe('A browser');
  });
});

describe('needsHomeScreen', () => {
  it('is Safari on an iPhone or iPad outside the Home Screen alone', () => {
    expect(needsHomeScreen(said(IPHONE_SAFARI, 5), false)).toBe(true);
    expect(needsHomeScreen(said(IPHONE_APP, 5), true)).toBe(false);
    expect(needsHomeScreen(said(SAFARI_MAC), false)).toBe(false);
  });

  it('knows an iPad that says it is a Mac', () => {
    expect(needsHomeScreen(said(SAFARI_MAC, 5), false)).toBe(true);
    expect(needsHomeScreen(said(SAFARI_MAC, 5), true)).toBe(false);
  });
});

describe('keyBytes and sameKey', () => {
  it('reads base64url, padded or not, as the bytes a subscription is made with', () => {
    expect([...keyBytes('AQID_-8')]).toEqual([1, 2, 3, 255, 239]);
    expect([...keyBytes('AQID')]).toEqual([1, 2, 3]);
    const key = keyBytes('AQID');
    expect(sameKey(new Uint8Array([1, 2, 3]).buffer, key)).toBe(true);
    expect(sameKey(new Uint8Array([1, 2, 4]).buffer, key)).toBe(false);
    expect(sameKey(new Uint8Array([1, 2]).buffer, key)).toBe(false);
    expect(sameKey(null, key)).toBe(false);
  });
});

describe('subscribed', () => {
  it('is the endpoint and keys a subscription gives, or nothing without them', () => {
    const endpoint = 'https://fcm.googleapis.com/fcm/send/a';
    const json = { endpoint, keys: { p256dh: 'pk', auth: 'au' } };
    expect(subscribed(json)).toEqual({ endpoint, p256dh: 'pk', auth: 'au' });
    expect(subscribed({ endpoint })).toBeNull();
    expect(subscribed({ keys: { p256dh: 'pk', auth: 'au' } })).toBeNull();
  });
});

describe('pushBrowser', () => {
  it('is nothing in a page with no push, as outside a secure context', () => {
    expect(pushBrowser(null)).toBeNull();
    // The tests' page has no service workers, as a page served over plain http has none.
    expect(pushBrowser(window)).toBeNull();
  });
});
