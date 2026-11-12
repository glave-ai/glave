import { DestroyRef, Service, inject } from '@angular/core';
import type { FileReadResult } from '@glave/client';

import { fromBase64url } from '../lib/passkeys';
import { SessionService } from './session.service';

/** The most files kept as addresses that nothing draws: past it, the one let go of first goes. */
const MOST_UNUSED = 48;

/** The most of a file read back, in bytes: more than any file glave keeps (10 MB). */
const MOST_BYTES = 32 * 1024 * 1024;

/** A file held as an address while something draws it. */
export interface HeldFile {
  /** Its address, once read. */
  readonly url: Promise<string>;
  /** Lets go of it, once: when nothing else holds it, it may go. */
  readonly release: () => void;
}

/** A file kept as an address, and how many hold it. */
interface Kept {
  readonly url: Promise<string>;
  readonly users: number;
}

/**
 * Files a thread holds, read back through the server (`file/read`), a piece at a time, and kept
 * as addresses the page made (`blob:`, which the app's Content Security Policy allows for
 * images): an image the agent made is drawn, and saved, from one read. An address is kept
 * while anything holds it; of those nothing holds, at most `MOST_UNUSED` are kept for when
 * they are drawn again, the one let go of first going first. One that could not be read is
 * read again when next asked for.
 */
@Service()
export class FilesService {
  private readonly session = inject(SessionService);
  /** Each file's address, by its thread and hash, the one held or let go of longest ago first. */
  private readonly kept = new Map<string, Kept>();

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
    const key = `${thread}/${sha256}`;
    const found = this.kept.get(key);
    const url = found?.url ?? this.read(thread, sha256, mediaType);
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

  /**
   * The file, read whole a piece at a time, as an address the page made. A piece that does not
   * end where the next starts, or a file larger than `MOST_BYTES`, is not read on.
   */
  private async read(thread: string, sha256: string, mediaType: string): Promise<string> {
    const unreadable = (): Error =>
      new Error($localize`:@@files.unreadable:This file could not be read`);
    const pieces: Uint8Array<ArrayBuffer>[] = [];
    let offset: number | null = 0;
    while (offset !== null) {
      const { piece }: FileReadResult = await this.session.call('file/read', {
        thread,
        sha256,
        ...(offset === 0 ? {} : { offset }),
      });
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
