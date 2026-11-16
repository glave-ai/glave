import { TestBed } from '@angular/core/testing';
import type { MadeFile } from '@glave/client';

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
    expect(made.byTurn()?.get(1)).toEqual([PLAN]);
  });
});
