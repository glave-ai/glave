import { ApiError, type ConnectionInfo, ErrorCode } from '@glave/client';

import { sentence, somethingWrong, unreached } from './text';

/** Failures that are glave's own, whose words are not for the person. */
const OWN: readonly number[] = [ErrorCode.INTERNAL, ErrorCode.PARSE, ErrorCode.INVALID_REQUEST];

/**
 * A new tab, which cannot reach this page; none where the browser opens none. Opened while a
 * click is handled, so no browser blocks it; once it reaches a sign-in on another origin,
 * glave's opener policy cuts it off from this page too, and this page from it.
 */
export function openedTab(document: Document): Window | null {
  const tab = document.defaultView?.open('', '_blank') ?? null;
  if (tab !== null) {
    tab.opener = null;
  }
  return tab;
}

/** Sends `tab` to `url`; whether it is open there, as one the person closed is not. */
export function sentTo(tab: Window | null, url: string): boolean {
  try {
    tab?.location.replace(url);
    return tab !== null && !tab.closed;
  } catch {
    return false;
  }
}

/** Where a service is: its address's host, or the whole address when it has none. */
export function serviceHost(connection: ConnectionInfo): string {
  try {
    return new URL(connection.url).host;
  } catch {
    return connection.url;
  }
}

/** What the person calls a service: its name in their settings, else where it is. */
export function serviceName(connection: ConnectionInfo): string {
  const name = connection.name?.trim() ?? '';
  return name === '' ? serviceHost(connection) : name;
}

/**
 * What to tell the person when a call about a connection failed: the server's own sentence for
 * a refusal, the plain failure for one of glave's own, and that glave could not be reached when
 * nothing answered.
 */
export function connectionProblem(error: unknown): string {
  if (!(error instanceof ApiError)) {
    return unreached();
  }
  return OWN.includes(error.code) ? somethingWrong() : sentence(error.message);
}
