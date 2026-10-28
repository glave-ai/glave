import { TestBed } from '@angular/core/testing';
import {
  ApiError,
  ErrorCode,
  type Notification,
  type RawEvent,
  type ThreadChanges,
} from '@glave/client';

import { FakeSession, provideFakeSession } from '../testing/fake-session';
import { event } from '../testing/fixtures';
import { ChangedFilesService } from './changed-files.service';
import { ThreadService } from './thread.service';

const WRITE = event(2, 'tool.call.requested', {
  call_id: 'c1',
  tool: 'write',
  arguments: { json: { path: 'notes.md', content: 'a\n' } },
});
const WROTE = event(3, 'tool.call.completed', { call_id: 'c1', is_error: false, output: [] });

const CHANGED: ThreadChanges = {
  files: [
    {
      path: 'notes.md',
      added: 1,
      removed: 0,
      changes: [{ kind: 'wrote', lines: [{ kind: 'added', text: 'a' }] }],
    },
  ],
  turns: [{ turn: 1, files: [] }],
  note: 'Changes made by commands or other tools are not listed.',
};

function recorded(raw: RawEvent): Notification {
  return { jsonrpc: '2.0', method: 'thread/event', params: { thread: raw.thread, event: raw } };
}

function make(fake: FakeSession): {
  readonly changed: ChangedFilesService;
  readonly thread: ThreadService;
} {
  TestBed.configureTestingModule({
    providers: [...provideFakeSession(fake), ThreadService, ChangedFilesService],
  });
  return { changed: TestBed.inject(ChangedFilesService), thread: TestBed.inject(ThreadService) };
}

/** Runs the effects, lets the calls they made answer, and runs the effects of those answers. */
async function settle(): Promise<void> {
  TestBed.tick();
  await new Promise((resolve) => setTimeout(resolve));
  TestBed.tick();
}

describe('ChangedFilesService', () => {
  beforeEach(() => {
    localStorage.clear();
  });

  it('asks nothing of a thread that never called a tool that changes files', async () => {
    const fake = new FakeSession().answer('thread/events', {
      events: [
        event(1, 'turn.started', {}),
        event(2, 'tool.call.requested', { call_id: 'c1', tool: 'bash', arguments: { json: {} } }),
        event(3, 'tool.call.completed', { call_id: 'c1', is_error: false, output: [] }),
      ],
    });
    const { changed, thread } = make(fake);
    await thread.open('thread-1');
    await settle();
    expect(fake.paramsOf('thread/changes')).toEqual([]);
    expect(changed.files()).toEqual([]);
    expect(changed.note()).toBe('');
    // Known as none once it has opened, so what its turns change next comes as the person watches.
    expect(changed.byTurn()).toEqual(new Map());
    expect(changed.known()).toBe(true);
  });

  it('draws the server’s changes as the thread opens, and asks again as a write ends', async () => {
    const fake = new FakeSession()
      .answer('thread/events', { events: [event(1, 'turn.started', {}), WRITE, WROTE] })
      .answer('thread/changes', { changes: CHANGED })
      .streams([
        recorded(event(4, 'turn.started', {})),
        recorded(
          event(5, 'tool.call.requested', { call_id: 'c2', tool: 'edit', arguments: { json: {} } }),
        ),
        recorded(event(6, 'tool.call.completed', { call_id: 'c2', is_error: true, output: [] })),
        recorded(event(7, 'turn.ended', {})),
      ]);
    const { changed, thread } = make(fake);
    const opening = thread.open('thread-1');
    // Not known while the thread opens, nor once it has, until the server says what it changed.
    TestBed.tick();
    expect(changed.byTurn()).toBeNull();
    await opening;
    expect([changed.byTurn(), changed.known()]).toEqual([null, false]);
    await settle();
    expect(changed.known()).toBe(true);
    expect(fake.paramsOf('thread/changes')).toEqual([{ thread: 'thread-1' }]);
    expect(changed.files()).toBe(CHANGED.files);
    expect(changed.byTurn()?.get(1)).toEqual([]);
    expect(changed.note()).toBe(CHANGED.note);
    await thread.send('Fix the typo');
    await settle();
    // The edit failed, but what came of it is the server's to say.
    expect(fake.paramsOf('thread/changes')).toHaveLength(2);
  });

  it('shows none when they cannot be read', async () => {
    const fake = new FakeSession()
      .answer('thread/events', { events: [WRITE, WROTE] })
      .answer(
        'thread/changes',
        new ApiError({ code: ErrorCode.INTERNAL, message: 'the server failed' }),
      );
    const { changed, thread } = make(fake);
    await thread.open('thread-1');
    await settle();
    expect(fake.paramsOf('thread/changes')).toHaveLength(1);
    expect(changed.files()).toEqual([]);
    // Known as none: the thread stands without them.
    expect(changed.known()).toBe(true);
  });
});
