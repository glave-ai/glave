import { tokenFromFragment } from '@glave/client';

/** Where the sign-in token is kept for this tab: never in a URL, a cookie or a log. */
export const TOKEN_KEY = 'glave-token';

/** What reading the token needs from the browser. */
export interface TokenPlace {
  readonly location: Pick<Location, 'hash' | 'pathname' | 'search'>;
  readonly history: Pick<History, 'replaceState'>;
  readonly storage: Pick<Storage, 'getItem' | 'setItem' | 'removeItem'> | null;
}

/**
 * The sign-in token: from the link `glave serve` printed (`#token=…`), which is then taken
 * out of the address bar and kept in this tab's session storage, or else from that storage.
 */
export function takeToken(place: TokenPlace): string | null {
  const fromLink = tokenFromFragment(place.location.hash);
  if (fromLink !== undefined) {
    place.history.replaceState(null, '', `${place.location.pathname}${place.location.search}`);
    try {
      place.storage?.setItem(TOKEN_KEY, fromLink);
    } catch {
      // Without storage the token lasts for this page.
    }
    return fromLink;
  }
  try {
    return place.storage?.getItem(TOKEN_KEY) ?? null;
  } catch {
    return null;
  }
}

/** Forgets the token kept in this tab. */
export function forgetToken(storage: Pick<Storage, 'removeItem'> | null): void {
  try {
    storage?.removeItem(TOKEN_KEY);
  } catch {
    // Nothing kept, nothing to forget.
  }
}
