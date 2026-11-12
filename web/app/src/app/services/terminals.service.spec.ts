import { TestBed } from '@angular/core/testing';
import { ApiError, ErrorCode } from '@glave/client';

import { FakeSession, provideFakeSession } from '../testing/fake-session';
import { TerminalsService } from './terminals.service';
import { ThreadService } from './thread.service';

const GLAVE = { id: 't1', folder: '/work/glave' } as const;
const SIZE = { cols: 80, rows: 24 } as const;

async function opened(fake: FakeSession): Promise<TerminalsService> {
  TestBed.configureTestingModule({
    providers: [
      ...provideFakeSession(fake.answer('thread/events', { events: [] })),
      ThreadService,
      TerminalsService,
    ],
  });
  await TestBed.inject(ThreadService).open('thread-1');
  return TestBed.inject(TerminalsService);
}

/** The params of each call of `method` the fake saw. */
function asked(fake: FakeSession, method: string): readonly Record<string, unknown>[] {
  return fake.calls
    .filter((call) => call.method === method)
    .map((call) => call.params as Record<string, unknown>);
}

describe('TerminalsService', () => {
  it('opens one where the thread has none, reads on from where it was, and types in order', async () => {
    let reads = 0;
    const fake = new FakeSession()
      .answer('terminal/list', { terminals: [] })
      .answer('terminal/open', { terminal: GLAVE })
      .answer('terminal/write', {})
      .answer('terminal/read', () => {
        reads += 1;
        return reads === 1
          ? { screen: { from: 0, next: 3, output: 'aGkh' } }
          : { screen: { from: 3, next: 3, output: '', ended: true } };
      });
    const service = await opened(fake);
    await service.start(SIZE);
    expect(asked(fake, 'terminal/open')).toEqual([{ thread: 'thread-1', size: SIZE }]);
    expect(service.shown()).toEqual(GLAVE);

    const printed: string[] = [];
    const print = (bytes: Uint8Array): void => {
      printed.push(new TextDecoder().decode(bytes));
    };
    await service.follow('t1', print, new AbortController().signal);
    expect(printed).toEqual(['hi!']);
    expect(asked(fake, 'terminal/read').map((params) => params['from'])).toEqual([0, 3]);
    expect(service.ended()).toBe(true);

    service.type('t1', new TextEncoder().encode('ls'));
    service.type('t1', new TextEncoder().encode('\r'));
    await new Promise((resolve) => setTimeout(resolve));
    expect(asked(fake, 'terminal/write').map((params) => params['input'])).toEqual([
      'bHM=',
      'DQ==',
    ]);
  });

  it('shows the terminals a thread has, the latest, and another once opened', async () => {
    const older = { id: 't0', folder: '/work/glave' };
    const fake = new FakeSession()
      .answer('terminal/list', { terminals: [older, GLAVE] })
      .answer('terminal/open', { terminal: { id: 't2', folder: '/work/glave' } })
      .answer('terminal/close', {})
      .answer('terminal/resize', {});
    const service = await opened(fake);
    await service.start(SIZE);
    expect(asked(fake, 'terminal/open')).toEqual([]);
    expect(service.shown()?.id).toBe('t1');
    await service.open(SIZE);
    expect(service.terminals().map((terminal) => terminal.id)).toEqual(['t0', 't1', 't2']);
    expect(service.shown()?.id).toBe('t2');
    service.show('t0');
    expect(service.shown()?.id).toBe('t0');
    await service.resize('t0', { cols: 100, rows: 30 });
    expect(asked(fake, 'terminal/resize')).toEqual([
      { thread: 'thread-1', terminal: 't0', size: { cols: 100, rows: 30 } },
    ]);
    await service.close('t0');
    expect(service.terminals().map((terminal) => terminal.id)).toEqual(['t1', 't2']);
    expect(service.shown()?.id).toBe('t2');
    expect(service.problem()).toBeNull();
  });

  it('says in the server’s words why a terminal cannot be opened', async () => {
    const refused = new ApiError({
      code: ErrorCode.NOT_ALLOWED,
      message: 'A terminal opens only on your own computer',
    });
    const fake = new FakeSession()
      .answer('terminal/list', { terminals: [] })
      .answer('terminal/open', () => refused);
    const service = await opened(fake);
    await service.start(SIZE);
    expect(service.shown()).toBeNull();
    expect(service.opening()).toBe(false);
    expect(service.problem()).toBe('A terminal opens only on your own computer.');
  });

  it('sends only the latest of the sizes a terminal took while one was being sent', async () => {
    const fake = new FakeSession();
    const give = fake.held('terminal/resize');
    const service = await opened(fake);
    const first = service.resize('t1', { cols: 81, rows: 24 });
    await new Promise((resolve) => setTimeout(resolve));
    void service.resize('t1', { cols: 90, rows: 24 });
    void service.resize('t1', { cols: 100, rows: 30 });
    give({});
    await new Promise((resolve) => setTimeout(resolve));
    give({});
    await first;
    expect(asked(fake, 'terminal/resize').map((params) => params['size'])).toEqual([
      { cols: 81, rows: 24 },
      { cols: 100, rows: 30 },
    ]);
  });

  it('types a paste larger than one write takes in pieces, in order', async () => {
    const fake = new FakeSession().answer('terminal/write', {});
    const service = await opened(fake);
    const paste = new Uint8Array(64 * 1024 * 2 + 3).fill(97);
    service.type('t1', paste);
    await new Promise((resolve) => setTimeout(resolve));
    const sent = asked(fake, 'terminal/write').map((params) => atob(String(params['input'])));
    expect(sent.map((piece) => piece.length)).toEqual([64 * 1024, 64 * 1024, 3]);
  });

  it('lists the terminals again after one is refused, and counts one gone as closed', async () => {
    const refused = new ApiError({
      code: ErrorCode.LIMIT,
      message: '4 terminals are already open in this chat; close one first',
    });
    const gone = new ApiError({ code: ErrorCode.NO_TERMINAL, message: 'no terminal' });
    let lists = 0;
    const fake = new FakeSession()
      .answer('terminal/list', () => {
        lists += 1;
        return { terminals: lists === 1 ? [] : [GLAVE] };
      })
      .answer('terminal/open', () => refused)
      .answer('terminal/close', () => gone);
    const service = await opened(fake);
    await service.start(SIZE);
    expect(lists).toBe(2);
    expect(service.shown()).toEqual(GLAVE);
    expect(service.problem()).toBe('4 terminals are already open in this chat; close one first.');
    await service.close('t1');
    expect(service.terminals()).toEqual([]);
    expect(service.problem()).toBe('4 terminals are already open in this chat; close one first.');
  });

  it('lists the terminals again every so often while it is watched, and not once stopped', async () => {
    let lists = 0;
    const fake = new FakeSession().answer('terminal/list', () => {
      lists += 1;
      return { terminals: [{ ...GLAVE, ended: true }] };
    });
    const service = await opened(fake);
    vi.useFakeTimers();
    const stop = new AbortController();
    const watching = service.watch(stop.signal);
    await vi.advanceTimersByTimeAsync(15_000);
    expect(lists).toBe(1);
    expect(service.shown()?.id).toBe('t1');
    expect(service.ended()).toBe(true);
    await vi.advanceTimersByTimeAsync(15_000);
    expect(lists).toBe(2);
    stop.abort();
    await watching;
    await vi.advanceTimersByTimeAsync(15_000);
    vi.useRealTimers();
    expect(lists).toBe(2);
  });

  it('reads again a while after a read fails, saying why meanwhile, and not while hidden', async () => {
    let reads = 0;
    const fake = new FakeSession().answer('terminal/read', () => {
      reads += 1;
      if (reads === 1) {
        throw new Error('unreachable');
      }
      return { screen: { from: 0, next: 0, output: '', ended: true } };
    });
    const service = await opened(fake);
    vi.useFakeTimers();
    const following = service.follow('t1', () => undefined, new AbortController().signal);
    await vi.advanceTimersByTimeAsync(0);
    expect(service.problem()).toBe('Something went wrong. Try again in a moment.');
    expect(reads).toBe(1);
    await vi.advanceTimersByTimeAsync(1_000);
    await following;
    vi.useRealTimers();
    expect(reads).toBe(2);
    expect(service.problem()).toBeNull();
    expect(service.ended()).toBe(false);

    // Hidden, it waits to be shown again before it reads.
    const hidden = vi.spyOn(document, 'hidden', 'get').mockReturnValue(true);
    const stop = new AbortController();
    const waiting = service.follow('t1', () => undefined, stop.signal);
    await new Promise((resolve) => setTimeout(resolve));
    expect(reads).toBe(2);
    stop.abort();
    await waiting;
    hidden.mockRestore();
  });
});
