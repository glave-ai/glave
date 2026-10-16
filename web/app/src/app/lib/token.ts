import { tokenFromFragment } from '@glave/client';

import { type Store, readStored, writeStored } from './storage';

/** Where the sign-in token is kept for this tab: never in a URL, a cookie or a log. */
export const TOKEN_KEY = 'glave-token';

/**
 * How a server's local token starts (`specs/session.md`, "Signing in"). The server takes it
 * only from this computer, so a session signed in with one is with a glave on this computer.
 */
const LOCAL_PREFIX = 'glave_local_';

/** Whether `token` is a server's local token, so the server runs on this computer. */
export function isLocalToken(token: string): boolean {
  return token.startsWith(LOCAL_PREFIX);
}

/** What reading the token needs from the browser. */
interface TokenPlace {
  readonly location: Pick<Location, 'hash' | 'pathname' | 'search'>;
  readonly history: Pick<History, 'replaceState'>;
  readonly storage: Store;
}

/**
 * The sign-in token: from the link `glave serve` printed (`#token=…`), which is then taken
 * out of the address bar and kept in this tab's session storage, or else from that storage.
 * Without storage the token lasts for this page.
 */
export function takeToken(place: TokenPlace): string | null {
  const fromLink = tokenFromFragment(place.location.hash);
  if (fromLink === undefined) {
    const kept = readStored(place.storage, TOKEN_KEY);
    return typeof kept === 'string' ? kept : null;
  }
  place.history.replaceState(null, '', `${place.location.pathname}${place.location.search}`);
  writeStored(place.storage, TOKEN_KEY, fromLink);
  return fromLink;
}
