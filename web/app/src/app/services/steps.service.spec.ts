import { TestBed } from '@angular/core/testing';
import { ApiError, ErrorCode, type Notification, type RawEvent } from '@glave/client';

import { FakeSession, provideFakeSession } from '../testing/fake-session';
import { event } from '../testing/fixtures';
import { StepsService } from './steps.service';
import { ThreadService } from './thread.service';

const RUN = event(2, 'tool.call.requested', {
  call_id: 'c1',
  tool: 'bash',
  arguments: { json: { command: 'ls' } },
});
const RAN = event(3, 'tool.call.completed', { call_id: 'c1', is_error: false, output: [] });

function recorded(raw: RawEvent): Notification {
  return { jsonrpc: '2.0', method: 'thread/event', params: { thread: raw.thread, event: raw } };
}

function make(fake: FakeSession): { readonly steps: StepsService; readonly thread: ThreadService } {
  TestBed.configureTestingModule({
    providers: [...provideFakeSession(fake), ThreadService, StepsService],
  });
  return { steps: TestBed.inject(StepsService), thread: TestBed.inject(ThreadService) };
}

/** Runs the effects, lets the calls they made answer, and runs the effects of those answers. */
async function settle(): Promise<void> {
  TestBed.tick();
  await new Promise((resolve) => setTimeout(resolve));
  TestBed.tick();
}

describe('StepsService', () => {
  beforeEach(() => {
    localStorage.clear();
  });

  it('asks nothing of a thread that never called a tool', async () => {
    const fake = new FakeSession().answer('thread/events', {
      events: [event(1, 'turn.started', {})],
    });
    const { steps, thread } = make(fake);
    await thread.open('thread-1');
    await settle();
    expect(fake.paramsOf('thread/steps')).toEqual([]);
    expect(steps.steps().runs.size).toBe(0);
  });

  it('finds each call’s run in the server’s line, and asks again as calls go on', async () => {
    const fake = new FakeSession()
      .answer('thread/events', { events: [event(1, 'turn.started', {}), RUN, RAN] })
      .answer('thread/steps', {
        steps: { runs: [{ first: 2, calls: ['c1'], sentence: 'Ran 1 command' }], replies: [4] },
      })
      .streams([
        recorded(event(5, 'turn.started', {})),
        recorded(
          event(6, 'tool.call.requested', { call_id: 'c2', tool: 'read', arguments: { json: {} } }),
        ),
        recorded(event(7, 'tool.call.completed', { call_id: 'c2', is_error: false, output: [] })),
        recorded(event(8, 'turn.ended', {})),
      ]);
    const { steps, thread } = make(fake);
    await thread.open('thread-1');
    await settle();
    expect(fake.paramsOf('thread/steps')).toEqual([{ thread: 'thread-1' }]);
    expect(steps.steps().runs.get('c1')).toEqual({ key: 'run-2', sentence: 'Ran 1 command' });
    expect(steps.steps().replies.has(4)).toBe(true);
    await thread.send('Read it');
    await settle();
    expect(fake.paramsOf('thread/steps').length).toBeGreaterThan(1);
  });

  it('tells no run when the steps cannot be read', async () => {
    const fake = new FakeSession()
      .answer('thread/events', { events: [RUN, RAN] })
      .answer('thread/steps', new ApiError({ code: ErrorCode.INTERNAL, message: 'failed' }));
    const { steps, thread } = make(fake);
    await thread.open('thread-1');
    await settle();
    expect(fake.paramsOf('thread/steps')).toHaveLength(1);
    expect(steps.steps().runs.size).toBe(0);
  });
});
