import type { RawEvent, ReplySources } from '@glave/client';

type Data = Readonly<Record<string, unknown>>;

function isRecord(value: unknown): value is Data {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

/**
 * Whether `event` records a page a call read: a `tool.call.completed` that did not fail, its
 * `structured` result holding the `page` (`web_fetch`'s).
 */
function readsPage(event: RawEvent): boolean {
  if (event.type !== 'tool.call.completed' || event.data['is_error'] === true) {
    return false;
  }
  const structured = event.data['structured'];
  return isRecord(structured) && isRecord(structured['page']);
}

/**
 * The `seq` of the latest end of a turn once a page was read, or `null` while none was: when it
 * changes, a reply may have drawn on pages it had not, and the sources are read again.
 */
export function lastSourcing(events: readonly RawEvent[]): number | null {
  let read = false;
  let last: number | null = null;
  for (const event of events) {
    read ||= readsPage(event);
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
