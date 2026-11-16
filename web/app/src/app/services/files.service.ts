import { DestroyRef, Service, type Signal, effect, inject, signal, untracked } from '@angular/core';
import type { FilePiece } from '@glave/client';

import { fromBase64url } from '../lib/passkeys';
import { SessionService } from './session.service';

/** The most files kept as addresses that nothing draws: past it, the one let go of first goes. */
const MOST_UNUSED = 48;

/** The most of a file read back, in bytes: more than any file glave keeps (10 MB). */
const MOST_BYTES = 32 * 1024 * 1024;

/** The most files read at once: past it, a read waits for one to end, the first asked first. */
const MOST_READING = 4;

/** A file held as an address while something draws it. */
export interface HeldFile {
  /** Its address, once read. */
  readonly url: Promise<string>;
  /** Lets go of it, once: when nothing else holds it, it may go. */
  readonly release: () => void;
}

/** A file as something draws it: its address once read, or why it could not be. */
export interface DrawnFile {
  /** Its address once read; `null` while it comes. */
  readonly url: Signal<string | null>;
  /** Why it can't be shown; `null` when nothing went wrong. */
  readonly failed: Signal<string | null>;
}

/**
 * The file `hold` holds, as what draws it shows it: held again whenever what `hold` reads
 * changes, and let go of when it does or what draws it goes; none while `hold` gives none. Call
 * it where `inject` may be called, as a field is made.
 */
export function drawn(hold: () => HeldFile | null): DrawnFile {
  const url = signal<string | null>(null);
  const failed = signal<string | null>(null);
  effect((onCleanup) => {
    const held = hold();
    untracked(() => {
      url.set(null);
      failed.set(null);
    });
    if (held === null) {
      return;
    }
    let live = true;
    onCleanup(() => {
      live = false;
      held.release();
    });
    held.url.then(
      (made) => {
        if (live) {
          url.set(made);
        }
      },
      (error: unknown) => {
        if (live) {
          failed.set(error instanceof Error ? error.message : String(error));
        }
      },
    );
  });
  return { url: url.asReadonly(), failed: failed.asReadonly() };
}

/** The piece of a file that starts at `offset`, as the server answers it. */
type ReadPiece = (offset: number) => Promise<{ readonly piece: FilePiece }>;

/** A file kept as an address, and how many hold it. */
interface Kept {
  readonly url: Promise<string>;
  readonly users: number;
}

/**
 * Files a thread holds (`file/read`), and items of the person's Library (`library/read`), read
 * back through the server a piece at a time, and kept as addresses the page made (`blob:`,
 * which the app's Content Security Policy allows for images): an image the agent made is drawn,
 * and saved, from one read. At most `MOST_READING` are read at once, so a page of many pictures
 * reads them a few at a time. An address is kept
 * while anything holds it; of those nothing holds, at most `MOST_UNUSED` are kept for when
 * they are drawn again, the one let go of first going first. One that could not be read is
 * read again when next asked for.
 */
@Service()
export class FilesService {
  private readonly session = inject(SessionService);
  /**
   * Each file's address, by its thread and hash or its Library item, the one held or let go of
   * longest ago first.
   */
  private readonly kept = new Map<string, Kept>();
  /** How many files are being read now. */
  private reading = 0;
  /** The reads waiting their turn, the first asked for first. */
  private readonly waiting: (() => void)[] = [];

  constructor() {
    inject(DestroyRef).onDestroy(() => {
      for (const { url } of this.kept.values()) {
        revoke(url);
      }
      this.kept.clear();
    });
  }

  /** The file `sha256` that `thread` holds, of `mediaType`, held as an address until released. */
  hold(thread: string, sha256: string, mediaType: string): HeldFile {
    return this.holdAs(`${thread}/${sha256}`, mediaType, (offset) =>
      this.session.call('file/read', { thread, sha256, ...(offset === 0 ? {} : { offset }) }),
    );
  }

  /**
   * The file of the Library item `asset`, in `project`, of `mediaType`, held as an address until
   * released.
   */
  holdAsset(project: string, asset: string, mediaType: string): HeldFile {
    return this.holdAs(`library/${project}/${asset}`, mediaType, (offset) =>
      this.session.call('library/read', { project, asset, ...(offset === 0 ? {} : { offset }) }),
    );
  }

  /** The file kept as `key`, read with `readPiece` the first time, held until released. */
  private holdAs(key: string, mediaType: string, readPiece: ReadPiece): HeldFile {
    const found = this.kept.get(key);
    const url = found?.url ?? this.inTurn(() => read(readPiece, mediaType));
    this.kept.delete(key);
    this.kept.set(key, { url, users: (found?.users ?? 0) + 1 });
    if (found === undefined) {
      url.catch(() => {
        if (this.kept.get(key)?.url === url) {
          this.kept.delete(key);
        }
      });
    }
    let held = true;
    return {
      url,
      release: () => {
        if (held) {
          held = false;
          this.letGo(key, url);
        }
      },
    };
  }

  /** `work`, once fewer than `MOST_READING` reads run; its turn passes on when it ends. */
  private async inTurn<T>(work: () => Promise<T>): Promise<T> {
    if (this.reading < MOST_READING) {
      this.reading += 1;
    } else {
      await new Promise<void>((go) => {
        this.waiting.push(go);
      });
    }
    try {
      return await work();
    } finally {
      const next = this.waiting.shift();
      if (next === undefined) {
        this.reading -= 1;
      } else {
        next();
      }
    }
  }

  /** One holder of `url`, kept as `key`, lets go of it; past `MOST_UNUSED` unheld, one goes. */
  private letGo(key: string, url: Promise<string>): void {
    const found = this.kept.get(key);
    if (found?.url !== url) {
      return;
    }
    this.kept.delete(key);
    this.kept.set(key, { url, users: found.users - 1 });
    let unused = Array.from(this.kept.values()).filter(({ users }) => users === 0).length;
    for (const [oldest, kept] of this.kept) {
      if (unused <= MOST_UNUSED) {
        break;
      }
      if (kept.users === 0) {
        this.kept.delete(oldest);
        revoke(kept.url);
        unused -= 1;
      }
    }
  }
}

/**
 * The file `readPiece` reads, read whole a piece at a time, as an address the page made. A piece
 * that does not end where the next starts, or a file larger than `MOST_BYTES`, is not read on.
 */
async function read(readPiece: ReadPiece, mediaType: string): Promise<string> {
  const unreadable = (): Error =>
    new Error($localize`:@@files.unreadable:This file could not be read`);
  const pieces: Uint8Array<ArrayBuffer>[] = [];
  let offset: number | null = 0;
  while (offset !== null) {
    const { piece } = await readPiece(offset);
    const bytes = fromBase64url(piece.data);
    const next: number | null = piece.next ?? null;
    const end: number = offset + (bytes?.length ?? 0);
    const moved = next === null || (next > offset && next === end);
    if (bytes === null || !moved || piece.bytes > MOST_BYTES || end > MOST_BYTES) {
      throw unreadable();
    }
    pieces.push(bytes);
    offset = next;
  }
  return URL.createObjectURL(new Blob(pieces, { type: mediaType }));
}

/** Lets go of the address `url` will be, once it is made. */
function revoke(url: Promise<string>): void {
  url.then(
    (made) => {
      URL.revokeObjectURL(made);
    },
    () => undefined,
  );
}
