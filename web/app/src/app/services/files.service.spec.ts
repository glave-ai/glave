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

describe('FilesService', () => {
  it('reads a file a piece at a time, once, and keeps it as an address the page made', async () => {
    const urls = objectUrls();
    const fake = new FakeSession().answer('file/read', inPieces('PNG!!'));
    const files = setUp(fake);
    const url = await files.url('t1', SHA, 'image/png');
    expect(url).toBe(urls.made[0]);
    expect(fake.paramsOf('file/read')).toEqual([
      { thread: 't1', sha256: SHA },
      { thread: 't1', sha256: SHA, offset: 2 },
      { thread: 't1', sha256: SHA, offset: 4 },
    ]);
    expect(await files.url('t1', SHA, 'image/png')).toBe(url);
    expect(fake.paramsOf('file/read')).toHaveLength(3);
  });

  it('reads one that could not be read again, and lets go of the oldest past its room', async () => {
    const urls = objectUrls();
    const gone = new ApiError({ code: ErrorCode.NO_FILE, message: 'this file is no longer here' });
    const fake = new FakeSession().answer('file/read', () => gone);
    const files = setUp(fake);
    await expect(files.url('t1', SHA, 'image/png')).rejects.toBe(gone);
    fake.answer('file/read', inPieces('ok'));
    expect(await files.url('t1', SHA, 'image/png')).toBe(urls.made[0]);
    for (let n = 0; n < 48; n += 1) {
      await files.url('t1', n.toString(16).padStart(64, '0'), 'image/png');
    }
    await Promise.resolve();
    expect(urls.revoked).toEqual([urls.made[0]]);
  });
});
