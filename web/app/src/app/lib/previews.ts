import type { CommandId, Port, ListedCommand } from '@glave/client';

/**
 * How long after the commands are read they are read again while the server previews and one
 * runs, in milliseconds, one after another, then no more until they change or the panel is shown
 * again: a server listens a moment after its command starts, or a minute later for a large
 * build, and another beside it later still, and nothing the thread records says when.
 */
export const LOOKS_AGAIN: readonly number[] = [1_000, 2_000, 4_000, 8_000, 15_000, 30_000, 60_000];

/** A preview of what a command serves at one port, as the server gives it. */
export interface Preview {
  readonly command: CommandId;
  readonly port: number;
  /** The server's own address, where a browser on this computer opens it. */
  readonly url: string;
  /** Whether a frame of the app shows it too. */
  readonly framed: boolean;
  /** Why a frame can't show it, in the server's words, where it opens in a new tab alone. */
  readonly unframed: string | null;
}

/** A preview's key among a thread's: its command, and the port it serves on. */
export function previewKey(preview: Pick<Preview, 'command' | 'port'>): string {
  return `${preview.command}:${String(preview.port)}`;
}

/** The previews of what `commands` serve, oldest command first, each at its ports in order. */
export function previewsOf(commands: readonly ListedCommand[]): readonly Preview[] {
  return commands.flatMap((command) =>
    (command.ports ?? []).flatMap((port: Port) =>
      typeof port.url === 'string'
        ? [
            {
              command: command.id,
              port: port.port,
              url: port.url,
              framed: port.framed,
              unframed: port.unframed ?? null,
            },
          ]
        : [],
    ),
  );
}
