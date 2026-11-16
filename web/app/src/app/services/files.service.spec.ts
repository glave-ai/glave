import { TestBed } from '@angular/core/testing';
import { ApiError, ErrorCode, type FileReadParams } from '@glave/client';

import { FakeSession, provideFakeSession } from '../testing/fake-session';
import { objectUrls } from '../testing/object-urls';
import { FilesService } from './files.service';

const SHA = 'a'.repeat(64);

function setUp(fake: FakeSession): FilesService {
  TestBed.configureTestingModule({ providers: [...provideFakeSession(fake)] });
  return TestBed.inject(FilesService);
}

/** A file of `bytes` read back in pieces of two bytes, as `file/read` answers each. */
function inPieces(bytes: string) {
  return (params: FileReadParams) => {
    const offset = params.offset ?? 0;
    const end = Math.min(offset + 2, bytes.length);
    return {
      piece: {
        media_type: 'image/png',
        bytes: bytes.length,
        offset,
        data: btoa(bytes.slice(offset, end)),
        ...(end < bytes.length ? { next: end } : {}),
      },
    };
  };
}

/** The `n`th of many files' hashes. */
function nth(n: number): string {
  return n.toString(16).padStart(64, '0');
}

describe('FilesService', () => {
  it('reads a file a piece at a time, once, and keeps it as an address the page made', async () => {
    const urls = objectUrls();
    const fake = new FakeSession().answer('file/read', inPieces('PNG!!'));
    const files = setUp(fake);
    const url = await files.hold('t1', SHA, 'image/png').url;
    expect(url).toBe(urls.made[0]);
    expect(fake.paramsOf('file/read')).toEqual([
      { thread: 't1', sha256: SHA },
      { thread: 't1', sha256: SHA, offset: 2 },
      { thread: 't1', sha256: SHA, offset: 4 },
    ]);
    expect(await files.hold('t1', SHA, 'image/png').url).toBe(url);
    expect(fake.paramsOf('file/read')).toHaveLength(3);
  });

  it('reads one that could not be read again when next asked for', async () => {
    const urls = objectUrls();
    const gone = new ApiError({ code: ErrorCode.NO_FILE, message: 'this file is no longer here' });
    const fake = new FakeSession().answer('file/read', () => gone);
    const files = setUp(fake);
    const failed = files.hold('t1', SHA, 'image/png');
    await expect(failed.url).rejects.toBe(gone);
    failed.release();
    fake.answer('file/read', inPieces('ok'));
    expect(await files.hold('t1', SHA, 'image/png').url).toBe(urls.made[0]);
  });

  it('never lets go of an address something draws, and keeps a few nothing does', async () => {
    const urls = objectUrls();
    const files = setUp(new FakeSession().answer('file/read', inPieces('ok')));
    const drawn = files.hold('t1', SHA, 'image/png');
    // The same image drawn twice, then once: still held.
    files.hold('t1', SHA, 'image/png').release();
    const many = Array.from({ length: 60 }, (_, n) => files.hold('t1', nth(n), 'image/png'));
    await Promise.all([drawn.url, ...many.map(({ url }) => url)]);
    await Promise.resolve();
    expect(urls.revoked).toEqual([]);
    for (const each of many) {
      each.release();
      each.release();
    }
    await Promise.resolve();
    // Of the 60 nothing draws, the first 12 let go of go; the one drawn all along stays.
    expect(urls.revoked).toEqual(urls.made.slice(1, 13));
    expect(await files.hold('t1', SHA, 'image/png').url).toBe(urls.made[0]);
  });

  it('reads no further than a piece that does not lead on, or a file larger than any kept', async () => {
    const urls = objectUrls();
    const piece = { media_type: 'image/png', bytes: 4, offset: 0, data: 'b2s=' };
    const fake = new FakeSession().answer('file/read', { piece: { ...piece, next: 0 } });
    const files = setUp(fake);
    const unreadable = 'This file could not be read';
    await expect(files.hold('t1', nth(1), 'image/png').url).rejects.toThrow(unreadable);
    fake.answer('file/read', { piece: { ...piece, next: 3 } });
    await expect(files.hold('t1', nth(2), 'image/png').url).rejects.toThrow(unreadable);
    fake.answer('file/read', { piece: { ...piece, bytes: 64 * 1024 * 1024, next: 2 } });
    await expect(files.hold('t1', nth(3), 'image/png').url).rejects.toThrow(unreadable);
    // Each was asked for once, and none was kept.
    expect(fake.paramsOf('file/read')).toHaveLength(3);
    expect(urls.made).toEqual([]);
  });
});
