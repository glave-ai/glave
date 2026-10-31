import type { Page, Route } from '@playwright/test';

import { posted } from './fake-records';

/** Answers `appearance/get` with `shades`, a real glave's, for a page on the fake API. */
export async function fakeAppearance(
  page: Page,
  shades: { readonly light: string; readonly dark: string },
): Promise<void> {
  await page.route('**/*', async (route: Route) => {
    const message = await posted(route);
    if (message === null) {
      return;
    }
    if (message.method !== 'appearance/get') {
      await route.fallback();
      return;
    }
    await route.fulfill({
      contentType: 'application/json',
      body: JSON.stringify({ jsonrpc: '2.0', id: message.id, result: { shades } }),
    });
  });
}

/** A real picture, one pixel, in base64, and as `brand/logo/get` says a logo. */
export const LOGO_PNG =
  'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M/wHwAEBgIApD5fRAAAAABJRU5ErkJggg==';
const LOGO = { sha256: 'a'.repeat(64), media_type: 'image/png', data: LOGO_PNG };

/**
 * Keeps an organization's brand as a server would for a page on the fake API: `brand/get`,
 * `brand/set`, `appearance/preview` and `appearance/get` of what is kept. What was sent to
 * `brand/set`, in order.
 */
export async function fakeBrand(page: Page): Promise<{ readonly kept: readonly object[] }> {
  const shades = { light: '#0f766e', dark: '#59c5b8' };
  interface Kept {
    name?: string;
    logo?: string;
    accent: unknown;
    locked: boolean;
  }
  const kept: Kept[] = [];
  const latest = (): Kept => kept.at(-1) ?? { accent: 'plain', locked: false };
  const answers: Record<string, (params: never) => object> = {
    'brand/get': () => ({ brand: latest() }),
    'brand/set': (params: Kept) => {
      kept.push(params);
      return { brand: params };
    },
    'brand/logo/upload': () => ({ sha256: LOGO.sha256 }),
    'brand/logo/get': () => ({ logo: LOGO }),
    'appearance/preview': () => ({ shades }),
    'appearance/get': () => ({ organization: latest(), shades }),
  };
  await page.route('**/*', async (route: Route) => {
    const message = await posted(route);
    if (message === null) {
      return;
    }
    const answer = answers[message.method];
    if (answer === undefined) {
      await route.fallback();
      return;
    }
    await route.fulfill({
      contentType: 'application/json',
      body: JSON.stringify({
        jsonrpc: '2.0',
        id: message.id,
        result: answer(message.params as never),
      }),
    });
  });
  return { kept };
}
