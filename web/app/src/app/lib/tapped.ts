/**
 * What glave's service worker asks an open window of the app to open when a notification is
 * tapped (`push-worker.js`): a page of the app, and the organization it is in.
 */

/** A page a tapped notification asks the app to open, in the organization `org`. */
export interface Tapped {
  readonly page: string;
  readonly org: string | null;
}

/** An id as the server writes an organization's. */
const ID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/u;

/** The pages a notification opens: a thread's, or the app's start. */
const PAGE = /^\/(t\/[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12})?$/u;

/** What `message`, posted by glave's service worker, asks the app to open; `null` for any other. */
export function tapped(message: unknown): Tapped | null {
  if (typeof message !== 'object' || message === null) {
    return null;
  }
  const { glave, page, org } = message as {
    readonly glave?: unknown;
    readonly page?: unknown;
    readonly org?: unknown;
  };
  if (glave !== 'open' || typeof page !== 'string' || !PAGE.test(page)) {
    return null;
  }
  return { page, org: typeof org === 'string' && ID.test(org) ? org : null };
}
