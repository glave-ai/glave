import {
  DOCUMENT,
  DestroyRef,
  Service,
  type Signal,
  effect,
  inject,
  signal,
  untracked,
} from '@angular/core';
import type { FilePiece } from '@glave/client';

import { UnreadableFile } from '../lib/files';
import { fromBase64url } from '../lib/passkeys';
import { SessionService } from './session.service';

/** The most files kept as addresses that nothing draws: past it, the one let go of first goes. */
const MOST_UNUSED = 48;

/**
 * The most bytes of files kept as addresses that nothing draws: a document a chat made is up to
 * 10 MB, so a few of them, not 48, are kept for when they are opened again.
 */
const MOST_UNUSED_BYTES = 32 * 1024 * 1024;

/** The most files opened in tabs of their own kept as addresses, the oldest let go of first. */
const MOST_IN_TABS = 4;

/** The most of a file read back, in bytes: more than any file glave keeps (10 MB). */
const MOST_BYTES = 32 * 1024 * 1024;

/**
 * How long a download's address is kept once the download has started, in milliseconds: a
 * phone's browser asks where to save it after the click, and reads it only then.
 */
const KEPT_FOR_SAVING = 60_000;

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

/** A file read back: its address, and its size. */
interface Read {
  readonly url: string;
  readonly bytes: number;
}

/** A file kept as an address, how many hold it, and its size once read (0 until then). */
interface Kept {
  readonly read: Promise<Read>;
  readonly users: number;
  readonly bytes: number;
}

/**
 * Files a thread holds (`file/read`), and items of the person's Library (`library/read`), read
 * back through the server a piece at a time, and kept as addresses the page made (`blob:`,
 * which the app's Content Security Policy allows for images): an image the agent made is drawn,
 * and saved, from one read. At most `MOST_READING` are read at once, so a page of many pictures
 * reads them a few at a time, and one whose turn comes once nothing holds it any more is not
 * read at all, so pictures scrolled past never keep those in view waiting. An address is kept
 * while anything holds it; of those nothing holds, at most `MOST_UNUSED`, and
 * `MOST_UNUSED_BYTES` in all, are kept for when they are drawn again, the one let go of first
 * going first. One that could not be read is read again when next asked for. A download, a
 * share and a file opened in a tab of its own each read a thread's file apart, and let go of its
 * address as soon as it is no longer needed.
 */
@Service()
export class FilesService {
  private readonly session = inject(SessionService);
  private readonly document = inject(DOCUMENT);
  /** The addresses of the files opened in tabs of their own, the oldest first. */
  private readonly inTabs: string[] = [];
  /**
   * Each file's address, by its thread and hash or its Library item, the one held or let go of
   * longest ago first.
   */
  private readonly kept = new Map<string, Kept>();
  /** How many files are being read now. */
  private reading = 0;
  /** The reads waiting their turn, the first asked for first. */
  private readonly waiting: (() => void)[] = [];
  /** Each download's address, with what lets go of it a minute after the download started. */
  private readonly saving = new Map<string, ReturnType<typeof setTimeout>>();

  constructor() {
    inject(DestroyRef).onDestroy(() => {
      for (const { read } of this.kept.values()) {
        revoke(read);
      }
      this.kept.clear();
      for (const url of this.inTabs.splice(0)) {
        URL.revokeObjectURL(url);
      }
      for (const [url, letGo] of this.saving) {
        clearTimeout(letGo);
        URL.revokeObjectURL(url);
      }
      this.saving.clear();
    });
  }

  /** The file `sha256` that `thread` holds, of `mediaType`, held as an address until released. */
  hold(thread: string, sha256: string, mediaType: string): HeldFile {
    return this.holdAs(`${thread}/${sha256}`, mediaType, this.fileRead(thread, sha256));
  }

  /** The file `sha256` that `thread` holds, of `mediaType`, read whole. */
  async blob(thread: string, sha256: string, mediaType: string): Promise<Blob> {
    return read(this.fileRead(thread, sha256), mediaType);
  }

  /**
   * Saves the file `sha256` that `thread` holds, of `mediaType`, to the person's drive under
   * `name`, as the browser saves a download, and lets go of its address a minute after the
   * download started; rejects, saying why, when it can't be read.
   */
  async save(thread: string, sha256: string, mediaType: string, name: string): Promise<void> {
    const url = URL.createObjectURL(await this.blob(thread, sha256, mediaType));
    const link = this.document.createElement('a');
    link.href = url;
    link.download = name;
    link.click();
    const letGo = setTimeout(() => {
      this.saving.delete(url);
      URL.revokeObjectURL(url);
    }, KEPT_FOR_SAVING);
    this.saving.set(url, letGo);
  }

  /**
   * Opens the file `sha256` that `thread` holds, of `mediaType`, in a tab of its own, where the
   * browser's own viewer draws it (a PDF pages through there). The tab opens as the person
   * asks, so the browser lets it, can't reach back to this page, and shows the file once it is
   * read; at most `MOST_IN_TABS` addresses are kept for such tabs. A browser that opens no tab
   * saves the file under `name` instead. Rejects, saying why, when it can't be read.
   */
  async openInTab(thread: string, sha256: string, mediaType: string, name: string): Promise<void> {
    const tab = this.document.defaultView?.open('', '_blank') ?? null;
    if (tab === null) {
      await this.save(thread, sha256, mediaType, name);
      return;
    }
    tab.opener = null;
    try {
      const url = URL.createObjectURL(await this.blob(thread, sha256, mediaType));
      this.inTabs.push(url);
      while (this.inTabs.length > MOST_IN_TABS) {
        URL.revokeObjectURL(this.inTabs.shift() ?? '');
      }
      tab.location.href = url;
    } catch (error: unknown) {
      tab.close();
      throw error;
    }
  }

  /** How the file `sha256` that `thread` holds is read, a piece at a time. */
  private fileRead(thread: string, sha256: string): ReadPiece {
    return (offset) =>
      this.session.call('file/read', { thread, sha256, ...(offset === 0 ? {} : { offset }) });
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
    const reading: Promise<Read> =
      found?.read ??
      this.inTurn(
        () => this.holds(key, reading),
        () => addressed(readPiece, mediaType),
      );
    this.kept.delete(key);
    const users = (found?.users ?? 0) + 1;
    this.kept.set(key, { read: reading, users, bytes: found?.bytes ?? 0 });
    if (found === undefined) {
      reading.then(
        ({ bytes }) => {
          const now = this.kept.get(key);
          if (now?.read === reading) {
            this.kept.set(key, { ...now, bytes });
          }
        },
        () => {
          if (this.kept.get(key)?.read === reading) {
            this.kept.delete(key);
          }
        },
      );
    }
    let held = true;
    return {
      url: reading.then(({ url }) => url),
      release: () => {
        if (held) {
          held = false;
          this.letGo(key, reading);
        }
      },
    };
  }

  /** Whether something holds the file kept as `key`, read as `reading`, now. */
  private holds(key: string, reading: Promise<Read>): boolean {
    const kept = this.kept.get(key);
    return kept?.read === reading && kept.users > 0;
  }

  /**
   * `work`, once fewer than `MOST_READING` reads run; its turn passes on when it ends. One that
   * had to wait is not done when its turn comes and it is no longer `wanted`: it fails, and the
   * turn passes on at once.
   */
  private async inTurn<T>(wanted: () => boolean, work: () => Promise<T>): Promise<T> {
    if (this.reading < MOST_READING) {
      this.reading += 1;
    } else {
      await new Promise<void>((go) => {
        this.waiting.push(go);
      });
      if (!wanted()) {
        this.passTurn();
        throw new Error('nothing holds this file any more');
      }
    }
    try {
      return await work();
    } finally {
      this.passTurn();
    }
  }

  /** Gives a read's turn to the first waiting, or frees it. */
  private passTurn(): void {
    const next = this.waiting.shift();
    if (next === undefined) {
      this.reading -= 1;
    } else {
      next();
    }
  }

  /**
   * One holder of `reading`, kept as `key`, lets go of it; past `MOST_UNUSED` or
   * `MOST_UNUSED_BYTES` unheld, the one let go of longest ago goes, and so on.
   */
  private letGo(key: string, reading: Promise<Read>): void {
    const found = this.kept.get(key);
    if (found?.read !== reading) {
      return;
    }
    this.kept.delete(key);
    this.kept.set(key, { ...found, users: found.users - 1 });
    const unused = Array.from(this.kept.values()).filter(({ users }) => users === 0);
    let count = unused.length;
    let bytes = unused.reduce((sum, kept) => sum + kept.bytes, 0);
    for (const [oldest, kept] of this.kept) {
      if (count <= MOST_UNUSED && bytes <= MOST_UNUSED_BYTES) {
        break;
      }
      if (kept.users === 0) {
        this.kept.delete(oldest);
        revoke(kept.read);
        count -= 1;
        bytes -= kept.bytes;
      }
    }
  }
}

/** The file `readPiece` reads, as an address the page made, with its size. */
async function addressed(readPiece: ReadPiece, mediaType: string): Promise<Read> {
  const blob = await read(readPiece, mediaType);
  return { url: URL.createObjectURL(blob), bytes: blob.size };
}

/**
 * The file `readPiece` reads, read whole a piece at a time. A piece that does not end where the
 * next starts, or a file larger than `MOST_BYTES`, is not read on.
 */
async function read(readPiece: ReadPiece, mediaType: string): Promise<Blob> {
  const unreadable = (): Error =>
    new UnreadableFile($localize`:@@files.unreadable:This file could not be read`);
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
  return new Blob(pieces, { type: mediaType });
}

/** Lets go of the address `reading` will make, once it is made. */
function revoke(reading: Promise<Read>): void {
  reading.then(
    ({ url }) => {
      URL.revokeObjectURL(url);
    },
    () => undefined,
  );
}
