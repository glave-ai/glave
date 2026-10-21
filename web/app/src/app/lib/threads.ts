import type { ThreadSummary } from '@glave/client';

/** The thread a URL of the app shows, if any: `/t/<id>`. */
export function threadFromUrl(url: string): string | null {
  const match = /^\/t\/([^/?#]+)/u.exec(url);
  return match?.[1] ?? null;
}

/** The workspace page a URL of the app shows (`tasks`, `memory`, `settings`), if any. */
export function pageFromUrl(url: string): string | null {
  const match = /^\/(tasks|memory|settings)(?:[/?#]|$)/u.exec(url);
  return match?.[1] ?? null;
}

/** A thread's title, or a stand-in until it has one. */
export function threadTitle(thread: Pick<ThreadSummary, 'title'>): string {
  const title = thread.title?.trim() ?? '';
  return title === '' ? 'New chat' : title;
}
