import type { Preview, RunningCommand } from '@glave/client';

/**
 * How long after the commands are read they are read again while one runs with no preview yet,
 * in milliseconds, one after another, then no more until they change or the panel is shown
 * again: a server listens a moment after its command starts, or a minute later for a large
 * build, and nothing the thread records says when.
 */
export const LOOKS_AGAIN: readonly number[] = [1_000, 2_000, 4_000, 8_000, 15_000, 30_000, 60_000];

/** A preview's key among a thread's: its command, and the port it serves on. */
export function previewKey(preview: Pick<Preview, 'command' | 'port'>): string {
  return `${preview.command}:${String(preview.port)}`;
}

/** Whether some of `commands` serve nothing `previews` shows yet. */
export function waitingForPreviews(
  commands: readonly RunningCommand[],
  previews: readonly Preview[],
): boolean {
  return commands.some((command) => !previews.some((preview) => preview.command === command.id));
}

/** The hosts a preview is served at: this computer's loopback addresses, as a URL writes them. */
const LOOPBACK: ReadonlySet<string> = new Set(['127.0.0.1', '[::1]']);

/** The one host whose pages glave's Content Security Policy lets it frame. */
const FRAMED = '127.0.0.1';

/**
 * Where the app may open `preview`, in a tab and in a frame, from a page at `page` (its own
 * host, as `location.hostname` says it).
 */
export interface Opened {
  /** Its address, for a link to a new tab; `null` for one the app opens nowhere. */
  readonly link: string | null;
  /**
   * Whether a frame of the app may show it too: at `127.0.0.1` alone, since a policy's sources
   * can't name IPv6's loopback, so a server listening there alone opens in a new tab only.
   */
  readonly framed: boolean;
}

/**
 * Where the app may open `preview` from a page at `page`: only a page on this computer's
 * loopback at a port of its own, plain HTTP, as the server gives every preview, and never at the
 * page's own host, where it would be the same site as glave. So nothing a reply or a page could
 * slip in is ever linked to or framed beside the app.
 */
export function opened(preview: Preview, page: string): Opened {
  let url: URL;
  try {
    url = new URL(preview.url);
  } catch {
    return { link: null, framed: false };
  }
  const loopback = url.protocol === 'http:' && LOOPBACK.has(url.hostname) && url.port !== '';
  const plain = url.username === '' && url.password === '' && url.hostname !== page;
  const link = loopback && plain ? url.href : null;
  return { link, framed: link !== null && url.hostname === FRAMED };
}
