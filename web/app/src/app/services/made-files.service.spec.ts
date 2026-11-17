import { TestBed } from '@angular/core/testing';
import {
  ApiError,
  ErrorCode,
  type MadeFile,
  type Notification,
  type RawEvent,
} from '@glave/client';

import { FakeSession, provideFakeSession } from '../testing/fake-session';
import { event } from '../testing/fixtures';
import { MadeFilesService } from './made-files.service';
import { ThreadService } from './thread.service';

const PLAN: MadeFile = {
  kind: 'pdf',
  name: 'plan',
  file_name: 'plan.pdf',
  media_type: 'application/pdf',
  bytes: 9,
  sha256: 'p'.repeat(64),
  previews: true,
  kept: 'Kept with this chat',
  sentence: 'Made a PDF: plan',
};

function recorded(raw: RawEvent): Notification {
  return { jsonrpc: '2.0', method: 'thread/event', params: { thread: raw.thread, event: raw } };
}

/** A turn's end that made the plan. */
function madePlan(seq: number): RawEvent {
  const blob = { sha256: PLAN.sha256, bytes: 9, media_type: 'application/pdf' };
  const made = [{ path: 'plan.pdf', bytes: 9, kept: { blob } }];
  return event(seq, 'turn.ended', { outcome: 'completed', made });
}

function make(fake: FakeSession): {
  readonly made: MadeFilesService;
  readonly thread: ThreadService;
} {
  TestBed.configureTestingModule({
    providers: [...provideFakeSession(fake), ThreadService, MadeFilesService],
  });
  return { made: TestBed.inject(MadeFilesService), thread: TestBed.inject(ThreadService) };
}

/** Runs the effects, lets the calls they made answer, and runs the effects of those answers. */
async function settle(): Promise<void> {
  TestBed.tick();
  await new Promise((resolve) => setTimeout(resolve));
  TestBed.tick();
}

describe('MadeFilesService', () => {
  beforeEach(() => {
    localStorage.clear();
  });

  it('asks nothing of a thread none of whose turns made a file', async () => {
    const fake = new FakeSession().answer('thread/events', {
      events: [event(1, 'turn.started', {}), event(2, 'turn.ended', { outcome: 'completed' })],
    });
    const { made, thread } = make(fake);
    await thread.open('thread-1');
    await settle();
    expect(fake.paramsOf('thread/files')).toEqual([]);
    expect(made.byTurn()).toBeNull();
  });

  it('reads the files each turn made once a turn’s end says it made one', async () => {
    const blob = { sha256: PLAN.sha256, bytes: 9, media_type: 'application/pdf' };
    const ended = { outcome: 'completed', made: [{ path: 'plan.pdf', bytes: 9, kept: { blob } }] };
    const fake = new FakeSession()
      .answer('thread/events', {
        events: [
          event(1, 'turn.started', {}),
          event(2, 'message.added', { role: 'user', parts: [{ type: 'text', text: 'Plan it' }] }),
          event(3, 'turn.ended', ended),
        ],
      })
      .answer('thread/files', { made: [{ turn: 1, files: [PLAN] }] });
    const { made, thread } = make(fake);
    await thread.open('thread-1');
    await settle();
    expect(fake.paramsOf('thread/files')).toEqual([{ thread: 'thread-1' }]);
    expect(made.byTurn()?.get(1)?.files).toEqual([PLAN]);
    expect(made.failed()).toBe(false);
  });

  it('reads them again only on what says a turn made more, never on anything else', async () => {
    const later = { turn: 1, made: [{ path: 'deck.pptx', bytes: 4, kept: 'secret' }] };
    const fake = new FakeSession()
      .answer('thread/events', { events: [event(1, 'turn.started', {}), madePlan(2)] })
      .answer('thread/files', { made: [{ turn: 1, files: [PLAN] }] })
      .streams([
        recorded(event(3, 'turn.started', {})),
        recorded(event(4, 'message.added', { role: 'assistant', parts: [] })),
        recorded(event(5, 'model.responded', {})),
        recorded(event(6, 'turn.ended', { outcome: 'completed' })),
      ]);
    const { thread } = make(fake);
    await thread.open('thread-1');
    await settle();
    expect(fake.paramsOf('thread/files')).toHaveLength(1);
    await thread.send('Thanks');
    await settle();
    // A turn that made nothing, and every event of it, asks nothing.
    expect(fake.paramsOf('thread/files')).toHaveLength(1);
    fake.streams([recorded(event(8, 'turn.started', {})), recorded(event(9, 'turn.made', later))]);
    await thread.send('And the deck?');
    await settle();
    expect(fake.paramsOf('thread/files')).toHaveLength(2);
  });

  it('reads one at a time, and once more after for every ask meanwhile', async () => {
    const fake = new FakeSession().answer('thread/events', { events: [madePlan(2)] });
    const give = fake.held('thread/files');
    const { made, thread } = make(fake);
    await thread.open('thread-1');
    TestBed.tick();
    void made.refresh();
    void made.refresh();
    expect(fake.paramsOf('thread/files')).toHaveLength(1);
    give({ made: [] });
    await settle();
    expect(fake.paramsOf('thread/files')).toHaveLength(2);
    give({ made: [{ turn: 1, files: [PLAN] }] });
    await settle();
    expect(fake.paramsOf('thread/files')).toHaveLength(2);
    expect(made.byTurn()?.get(1)?.files).toEqual([PLAN]);
  });

  it('keeps what it read when a later read fails, and says it failed', async () => {
    const fake = new FakeSession()
      .answer('thread/events', { events: [event(1, 'turn.started', {}), madePlan(2)] })
      .answer('thread/files', { made: [{ turn: 1, files: [PLAN] }] });
    const { made, thread } = make(fake);
    await thread.open('thread-1');
    await settle();
    fake.answer('thread/files', new ApiError({ code: ErrorCode.NO_FILE, message: 'gone' }));
    await made.refresh();
    expect(made.failed()).toBe(true);
    expect(made.byTurn()?.get(1)?.files).toEqual([PLAN]);
  });

  it('says it couldn’t read them, and what later look-overs said each turn made', async () => {
    const later = { turn: 1, made: [{ path: 'plan.pdf', bytes: 9, kept: 'too_large' }] };
    const fake = new FakeSession()
      .answer('thread/events', {
        events: [
          event(1, 'turn.started', {}),
          event(2, 'turn.ended', { outcome: 'completed' }),
          event(3, 'turn.made', later),
        ],
      })
      .answer('thread/files', new ApiError({ code: ErrorCode.NO_FILE, message: 'gone' }));
    const { made, thread } = make(fake);
    await thread.open('thread-1');
    await settle();
    expect(made.failed()).toBe(true);
    expect(made.byTurn()).toBeNull();
    expect(thread.made.later().get(1)?.files).toEqual(later.made);
  });
});
