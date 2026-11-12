import { DestroyRef, Service, inject } from '@angular/core';
import type { FileReadResult } from '@glave/client';

import { fromBase64url } from '../lib/passkeys';
import { SessionService } from './session.service';

/** The most files kept as addresses at once: past it, the one used longest ago goes. */
const MOST_KEPT = 48;

/**
 * Files a thread holds, read back through the server (`file/read`), a piece at a time, and kept
 * as addresses the page made (`blob:`, which the app's Content Security Policy allows for
 * images): an image the agent made is drawn, and saved, from one read. At most `MOST_KEPT` are
 * kept, the one used longest ago let go of first; one that could not be read is read again
 * when next asked for.
 */
@Service()
export class FilesService {
  private readonly session = inject(SessionService);
  /** Each file's address, by its thread and hash, the one used longest ago first. */
  private readonly kept = new Map<string, Promise<string>>();

  constructor() {
    inject(DestroyRef).onDestroy(() => {
      for (const url of this.kept.values()) {
        release(url);
      }
      this.kept.clear();
    });
  }

  /** An address of the file `sha256` that `thread` holds, of `mediaType`. */
  url(thread: string, sha256: string, mediaType: string): Promise<string> {
    const key = `${thread}/${sha256}`;
    const found = this.kept.get(key);
    if (found !== undefined) {
      this.kept.delete(key);
      this.kept.set(key, found);
      return found;
    }
    const reading = this.read(thread, sha256, mediaType);
    this.kept.set(key, reading);
    reading.catch(() => {
      if (this.kept.get(key) === reading) {
        this.kept.delete(key);
      }
    });
    for (const [oldest, url] of this.kept) {
      if (this.kept.size <= MOST_KEPT) {
        break;
      }
      this.kept.delete(oldest);
      release(url);
    }
    return reading;
  }

  /** The file, read whole a piece at a time, as an address the page made. */
  private async read(thread: string, sha256: string, mediaType: string): Promise<string> {
    const pieces: Uint8Array<ArrayBuffer>[] = [];
    let offset: number | null = 0;
    while (offset !== null) {
      const { piece }: FileReadResult = await this.session.call('file/read', {
        thread,
        sha256,
        ...(offset === 0 ? {} : { offset }),
      });
      const bytes = fromBase64url(piece.data);
      if (bytes === null) {
        throw new Error($localize`:@@files.unreadable:This file could not be read`);
      }
      pieces.push(bytes);
      offset = piece.next ?? null;
    }
    return URL.createObjectURL(new Blob(pieces, { type: mediaType }));
  }
}

/** Lets go of the address `url` will be, once it is made. */
function release(url: Promise<string>): void {
  url.then(
    (made) => {
      URL.revokeObjectURL(made);
    },
    () => undefined,
  );
}
