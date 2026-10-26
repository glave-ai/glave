import type { RawEvent, ReplySources } from '@glave/client';

/** The tool that reads pages, the one whose calls can make a reply's sources. */
const WEB_FETCH = 'web_fetch';

/** Whether `event` asks to read a page: a call of `web_fetch`, which may or may not load it. */
function fetches(event: RawEvent): boolean {
  return event.type === 'tool.call.requested' && event.data['tool'] === WEB_FETCH;
}

/**
 * The `seq` of the latest end of a turn once the thread asked to read a page, or `null` while it
 * had not: when it changes, a reply may have drawn on pages it had not, and the sources are read
 * again. What a call read, if anything, is the server's to say.
 */
export function lastSourcing(events: readonly RawEvent[]): number | null {
  let read = false;
  let last: number | null = null;
  for (const event of events) {
    read ||= fetches(event);
    if (read && event.type === 'turn.ended') {
      last = event.seq;
    }
  }
  return last;
}

/** A page a reply drew on, as its link shows it. */
export interface SourceLink {
  readonly href: string;
  /** The title the page gives itself; `null` when it gives none, and its address names it. */
  readonly title: string | null;
  /** Where it is, for the reader: its host and path. */
  readonly address: string;
}

/** Where `url` is, for the reader: its host, path and query, without the scheme or a lone `/`. */
export function shortAddress(url: string): string {
  try {
    const parsed = new URL(url);
    const path = parsed.pathname === '/' ? '' : parsed.pathname;
    return `${parsed.host}${path}${parsed.search}`;
  } catch {
    return url;
  }
}

/** The pages `sources` names, as links. */
export function sourceLinks(sources: ReplySources): readonly SourceLink[] {
  return sources.pages.map((page) => ({
    href: page.url,
    title: page.title ?? null,
    address: shortAddress(page.url),
  }));
}
