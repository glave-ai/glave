import type { ThreadSummary } from '@glave/client';
import type { GlzBadgeVariant, GlzTone } from '@glave/glaze';

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

/**
 * What a thread's line shows besides its title. In the side nav it stays quiet (`variant`): a
 * dot for what needs the person, the word alone, muted, for what runs.
 */
export interface ThreadMark {
  readonly text: string;
  readonly tone: GlzTone;
  readonly variant: GlzBadgeVariant;
}

/** The mark for a thread's status: running, or waiting for the person; none when idle. */
export function threadMark(thread: Pick<ThreadSummary, 'status'>): ThreadMark | null {
  switch (thread.status) {
    case 'running':
      return { text: 'running', tone: 'neutral', variant: 'plain' };
    case 'waiting':
      return { text: 'needs you', tone: 'accent', variant: 'dot' };
    case 'idle':
      return null;
  }
}

/** A thread's title, or a stand-in until it has one. */
export function threadTitle(thread: Pick<ThreadSummary, 'title'>): string {
  const title = thread.title?.trim() ?? '';
  return title === '' ? 'New chat' : title;
}
