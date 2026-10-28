import { TestBed } from '@angular/core/testing';
import {
  ApiError,
  ErrorCode,
  type Notification,
  type RawEvent,
  type ReplySources,
} from '@glave/client';

import { FakeSession, provideFakeSession } from '../testing/fake-session';
import { event } from '../testing/fixtures';
import { SourcesService } from './sources.service';
import { ThreadService } from './thread.service';

const READ = event(5, 'tool.call.requested', {
  call_id: 'c1',
  tool: 'web_fetch',
  arguments: { json: { url: 'https://tokio.rs/blog' } },
});

const SOURCED: ReplySources = {
  reply: 6,
  sentence: 'Sources: 1 page',
  pages: [{ url: 'https://tokio.rs/blog', title: 'Tokio blog' }],
};

function recorded(raw: RawEvent): Notification {
  return { jsonrpc: '2.0', method: 'thread/event', params: { thread: raw.thread, event: raw } };
}

function make(fake: FakeSession): {
  readonly sources: SourcesService;
  readonly thread: ThreadService;
} {
  TestBed.configureTestingModule({
    providers: [...provideFakeSession(fake), ThreadService, SourcesService],
  });
  return { sources: TestBed.inject(SourcesService), thread: TestBed.inject(ThreadService) };
}

/** Runs the effects, lets the calls they made answer, and runs the effects of those answers. */
async function settle(): Promise<void> {
  TestBed.tick();
  await new Promise((resolve) => setTimeout(resolve));
  TestBed.tick();
}

describe('SourcesService', () => {
  beforeEach(() => {
    localStorage.clear();
  });

  it('asks nothing of a thread that never read a page', async () => {
    const fake = new FakeSession().answer('thread/events', {
      events: [event(1, 'turn.started', {}), event(2, 'turn.ended', { outcome: 'completed' })],
    });
    const { sources, thread } = make(fake);
    await thread.open('thread-1');
    await settle();
    expect(fake.paramsOf('thread/sources')).toEqual([]);
    expect(sources.byReply().size).toBe(0);
    // Nothing to read, so what the thread is drawn with is known.
    expect(sources.known()).toBe(true);
  });

  it('reads a thread’s sources as it opens, and again when a turn that read a page ends', async () => {
    const history = [event(1, 'turn.started', {}), READ, event(7, 'turn.ended', {})];
    const fake = new FakeSession()
      .answer('thread/events', { events: history })
      .answer('thread/sources', { sources: [SOURCED] })
      .streams([recorded(event(8, 'turn.started', {})), recorded(event(9, 'turn.ended', {}))]);
    const { sources, thread } = make(fake);
    await thread.open('thread-1');
    // Not known once it has opened, until the server answers.
    expect(sources.known()).toBe(false);
    await settle();
    expect(fake.paramsOf('thread/sources')).toEqual([{ thread: 'thread-1' }]);
    expect(sources.byReply().get(6)).toEqual(SOURCED);
    expect(sources.known()).toBe(true);
    await thread.send('And since?');
    await settle();
    expect(fake.paramsOf('thread/sources')).toHaveLength(2);
  });

  it('asks once for a history read in pages, after the last', async () => {
    const pages = [
      { events: [READ, event(7, 'turn.ended', {})], next: 7 },
      { events: [event(8, 'turn.started', {}), event(9, 'turn.ended', {})] },
    ];
    const fake = new FakeSession().answer('thread/events', (params) =>
      (params as { readonly after?: number }).after === undefined ? pages[0] : pages[1],
    );
    const { thread } = make(fake);
    const opened = thread.open('thread-1');
    // While the first page is in and the second on its way, nothing is asked.
    await Promise.resolve();
    TestBed.tick();
    await opened;
    await settle();
    expect(fake.paramsOf('thread/events')).toHaveLength(2);
    expect(fake.paramsOf('thread/sources')).toEqual([{ thread: 'thread-1' }]);
  });

  it('shows none when they cannot be read', async () => {
    const fake = new FakeSession()
      .answer('thread/events', { events: [READ, event(7, 'turn.ended', {})] })
      .answer(
        'thread/sources',
        new ApiError({ code: ErrorCode.INTERNAL, message: 'the server failed' }),
      );
    const { sources, thread } = make(fake);
    await thread.open('thread-1');
    await settle();
    expect(fake.paramsOf('thread/sources')).toHaveLength(1);
    expect(sources.byReply().size).toBe(0);
    // Known as none: the replies stand without them.
    expect(sources.known()).toBe(true);
  });
});
