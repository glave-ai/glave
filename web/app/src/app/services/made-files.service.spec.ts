import { TestBed } from '@angular/core/testing';
import type { MadeFile } from '@glave/client';

import { FakeSession, provideFakeSession } from '../testing/fake-session';
import { event } from '../testing/fixtures';
import { MadeFilesService } from './made-files.service';
import { ThreadService } from './thread.service';

const PLAN: MadeFile = {
  seq: 3,
  sha256: 'p'.repeat(64),
  name: 'plan.pdf',
  title: 'plan',
  kind: 'pdf',
  media_type: 'application/pdf',
  bytes: 9,
  kept: 'Kept with this chat',
};

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

  it('asks nothing of a thread that ran no call that can make a file', async () => {
    const fake = new FakeSession().answer('thread/events', {
      events: [event(1, 'turn.started', {}), event(2, 'turn.ended', { outcome: 'completed' })],
    });
    const { made, thread } = make(fake);
    await thread.open('thread-1');
    await settle();
    expect(fake.paramsOf('thread/files')).toEqual([]);
    expect(made.byTurn().size).toBe(0);
  });

  it('reads the files each turn made once a call that can make one ended', async () => {
    const history = [
      event(1, 'turn.started', {}),
      event(2, 'tool.call.requested', { call_id: 'c1', tool: 'bash', arguments: { json: {} } }),
      event(3, 'tool.call.completed', { call_id: 'c1', is_error: false, output: [] }),
    ];
    const fake = new FakeSession()
      .answer('thread/events', { events: history })
      .answer('thread/files', { made: [{ turn: 1, files: [PLAN] }] });
    const { made, thread } = make(fake);
    await thread.open('thread-1');
    await settle();
    expect(fake.paramsOf('thread/files')).toEqual([{ thread: 'thread-1' }]);
    expect(made.byTurn().get(1)).toEqual([PLAN]);
  });
});
