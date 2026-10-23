import { codeFromFragment } from '@glave/client';

import { type Store, readStored, writeStored } from './storage';

/**
 * Where this browser keeps its half of a session, for every tab of this origin alone: never in
 * a URL or a log. The server keeps the other half in a cookie no script reads.
 */
export const TOKEN_KEY = 'glave-session';

/**
 * A browser session (the half the app presents), and the id of the server that made it: it is
 * presented to no other.
 */
export interface BrowserSession {
  readonly token: string;
  readonly server: string;
}

/**
 * How a browser session starts (`specs/one-server.md`, "Finding it"). A glave makes one only
 * for the person on its own computer, and takes it only from there, so a tab signed in with
 * one is with a glave on this computer.
 */
const SESSION_PREFIX = 'glave_session_';

/** Whether `token` is a browser session, so the server runs on this computer. */
export function isLocalToken(token: string): boolean {
  return token.startsWith(SESSION_PREFIX);
}

/** What reading the link needs from the browser. */
interface LinkPlace {
  readonly location: Pick<Location, 'hash' | 'pathname' | 'search'>;
  readonly history: Pick<History, 'replaceState'>;
}

/**
 * The one-time code in the link glave opened or printed: a sign-in link's (`#code=…`) or a
 * setup link's (`#setup=…`), taken out of the address bar at once, so it is in no history
 * entry; `null` when the link has none.
 */
export function takeCode(place: LinkPlace, link: 'code' | 'setup'): string | null {
  const code = codeFromFragment(place.location.hash, link);
  if (code === undefined) {
    return null;
  }
  place.history.replaceState(null, '', `${place.location.pathname}${place.location.search}`);
  return code;
}

/** The session this browser kept, if it kept one. */
export function readSession(storage: Store): BrowserSession | null {
  const kept = readStored(storage, TOKEN_KEY);
  return isSession(kept) ? kept : null;
}

/** Keeps `session` for this browser; without storage it lasts for this page. */
export function keepSession(storage: Store, session: BrowserSession): void {
  writeStored(storage, TOKEN_KEY, session);
}

/**
 * The session to present to the server whose `server/discover` answered the id `server`: only
 * one that server made. Another server, or one that names none, gets nothing.
 */
export function sessionFor(
  kept: BrowserSession | null,
  server: string | undefined,
): BrowserSession | null {
  return kept !== null && server !== undefined && kept.server === server ? kept : null;
}

function isSession(value: unknown): value is BrowserSession {
  if (typeof value !== 'object' || value === null) {
    return false;
  }
  const fields = value as Readonly<Record<string, unknown>>;
  return typeof fields['token'] === 'string' && typeof fields['server'] === 'string';
}
