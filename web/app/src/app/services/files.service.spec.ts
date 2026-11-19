import { TestBed } from '@angular/core/testing';
import { ApiError, ErrorCode, type FileReadParams } from '@glave/client';

import { settle } from '@glave/glaze/testing/settle';
import { onTestFinished } from 'vitest';

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
  return (params: Pick<FileReadParams, 'offset'>) => {
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

/** A session whose `file/read`s each wait for the test to answer them, in `answers`, in order. */
function answeredByTheTest(): {
  readonly fake: FakeSession;
  readonly answers: readonly ((answer: object) => void)[];
} {
  const answers: ((answer: object) => void)[] = [];
  const fake = new FakeSession().answer(
    'file/read',
    () =>
      new Promise<object>((resolve) => {
        answers.push(resolve);
      }),
  );
  return { fake, answers };
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

  it("reads a Library item's file the same way, by the item", async () => {
    const urls = objectUrls();
    const fake = new FakeSession().answer('library/read', inPieces('PNG!'));
    const files = setUp(fake);
    expect(await files.holdAsset('p1', 'fox', 'image/png').url).toBe(urls.made[0]);
    expect(fake.paramsOf('library/read')).toEqual([
      { project: 'p1', asset: 'fox' },
      { project: 'p1', asset: 'fox', offset: 2 },
    ]);
    expect(await files.holdAsset('p1', 'fox', 'image/png').url).toBe(urls.made[0]);
    expect(fake.paramsOf('library/read')).toHaveLength(2);
  });

  it('reads four files at once, the next as one ends', async () => {
    objectUrls();
    const { fake, answers } = answeredByTheTest();
    const files = setUp(fake);
    const held = [0, 1, 2, 3, 4, 5].map((n) => files.hold('t1', nth(n), 'image/png'));
    expect(fake.paramsOf('file/read')).toHaveLength(4);
    answers[0]?.(inPieces('ok')({}));
    await held[0]?.url;
    await settle();
    expect(fake.paramsOf('file/read')).toHaveLength(5);
  });

  it('does not read a file nothing holds any more once its turn comes, and reads it if held again', async () => {
    objectUrls();
    const { fake, answers } = answeredByTheTest();
    const files = setUp(fake);
    const held = [0, 1, 2, 3, 4].map((n) => files.hold('t1', nth(n), 'image/png'));
    expect(fake.paramsOf('file/read')).toHaveLength(4);
    // Scrolled past before its turn: let go of while it waits.
    const passed = held[4];
    passed?.release();
    answers[0]?.(inPieces('ok')({}));
    await held[0]?.url;
    await expect(passed?.url).rejects.toThrow();
    await settle();
    expect(fake.paramsOf('file/read')).toHaveLength(4);
    // Held again, it is read.
    void files.hold('t1', nth(4), 'image/png').url;
    await settle();
    expect(fake.paramsOf('file/read')).toHaveLength(5);
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

  it('keeps fewer large files nothing draws: at most 32 MB of them', async () => {
    const urls = objectUrls();
    const twelve = btoa('a'.repeat(12 * 1024 * 1024));
    const whole = { media_type: 'application/pdf', bytes: 12 * 1024 * 1024, offset: 0 };
    const files = setUp(
      new FakeSession().answer('file/read', { piece: { ...whole, data: twelve } }),
    );
    const held = [1, 2, 3].map((n) => files.hold('t1', nth(n), 'application/pdf'));
    await Promise.all(held.map(({ url }) => url));
    await Promise.resolve();
    for (const each of held) {
      each.release();
    }
    await Promise.resolve();
    // 36 MB nothing draws: the one let go of first goes.
    expect(urls.revoked).toEqual(urls.made.slice(0, 1));
  });

  it('saves a file under its name, and lets go of its address a minute after', async () => {
    const urls = objectUrls();
    vi.useFakeTimers({ toFake: ['setTimeout'] });
    onTestFinished(() => {
      vi.useRealTimers();
    });
    const files = setUp(new FakeSession().answer('file/read', inPieces('ok')));
    const clicked: string[] = [];
    vi.spyOn(HTMLAnchorElement.prototype, 'click').mockImplementation(function (
      this: HTMLAnchorElement,
    ) {
      clicked.push(`${this.download} ${this.href}`);
    });
    await files.save('t1', SHA, 'text/csv', 'budget.csv');
    expect(clicked).toEqual([`budget.csv ${urls.made[0] ?? ''}`]);
    vi.advanceTimersByTime(59_000);
    expect(urls.revoked).toEqual([]);
    vi.advanceTimersByTime(1_000);
    expect(urls.revoked).toEqual(urls.made);
  });

  it('opens a file in a tab that can’t reach back to the page, or saves it where none opens', async () => {
    const urls = objectUrls();
    const files = setUp(new FakeSession().answer('file/read', inPieces('ok')));
    const tab = { location: { href: '' }, opener: {} as unknown, close: vi.fn() };
    const open = vi.spyOn(window, 'open').mockReturnValue(tab as unknown as Window);
    onTestFinished(() => {
      open.mockRestore();
    });
    await files.openInTab('t1', SHA, 'application/pdf', 'plan.pdf');
    expect(tab.opener).toBeNull();
    expect(tab.location.href).toBe(urls.made[0]);
    open.mockReturnValue(null);
    const saved: string[] = [];
    vi.spyOn(HTMLAnchorElement.prototype, 'click').mockImplementation(function (
      this: HTMLAnchorElement,
    ) {
      saved.push(this.download);
    });
    await files.openInTab('t1', SHA, 'application/pdf', 'plan.pdf');
    expect(saved).toEqual(['plan.pdf']);
  });

  it('lets go of a download’s address as the page goes, before its minute is up', async () => {
    const urls = objectUrls();
    vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout'] });
    onTestFinished(() => {
      vi.useRealTimers();
    });
    const files = setUp(new FakeSession().answer('file/read', inPieces('ok')));
    vi.spyOn(HTMLAnchorElement.prototype, 'click').mockImplementation(() => undefined);
    await files.save('t1', SHA, 'text/csv', 'budget.csv');
    TestBed.resetTestingModule();
    expect(urls.revoked).toEqual(urls.made);
    vi.runAllTimers();
    expect(urls.revoked).toEqual(urls.made);
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
