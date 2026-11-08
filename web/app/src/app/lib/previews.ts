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

/**
 * The address a frame may show for `preview`, or `null`: only a page on this computer's
 * loopback at a port of its own, plain HTTP, as the server opens every preview, so nothing a
 * reply or a page could slip in is ever framed beside the app.
 */
export function frameable(preview: Preview): string | null {
  let url: URL;
  try {
    url = new URL(preview.url);
  } catch {
    return null;
  }
  const own = url.protocol === 'http:' && url.hostname === '127.0.0.1' && url.port !== '';
  return own && url.username === '' && url.password === '' ? url.href : null;
}
