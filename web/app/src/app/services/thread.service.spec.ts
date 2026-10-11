import { TestBed } from '@angular/core/testing';

import { FakeSession, provideFakeSession } from '../testing/fake-session';
import { event } from '../testing/fixtures';
import { ThreadService } from './thread.service';

function make(fake: FakeSession): ThreadService {
  TestBed.configureTestingModule({ providers: [...provideFakeSession(fake), ThreadService] });
  return TestBed.inject(ThreadService);
}

describe('ThreadService', () => {
  it('reads every page of a thread', async () => {
    const fake = new FakeSession().answer('thread/events', (params) =>
      (params as { after?: number }).after === undefined
        ? {
            events: [
              event(1, 'message.added', { role: 'user', parts: [{ type: 'text', text: 'a' }] }),
            ],
            next: 1,
          }
        : {
            events: [
              event(2, 'message.added', {
                role: 'assistant',
                parts: [{ type: 'text', text: 'b' }],
              }),
            ],
          },
    );
    const service = make(fake);
    await service.open('thread-1');
    expect(service.items().map((item) => item.kind)).toEqual(['message', 'message']);
    expect(fake.paramsOf('thread/events')).toEqual([
      { thread: 'thread-1' },
      { thread: 'thread-1', after: 1 },
    ]);
  });

  it('streams a turn: the draft grows, then the recorded message replaces it', async () => {
    const fake = new FakeSession().answer('thread/events', { events: [] }).streams([
      {
        jsonrpc: '2.0',
        method: 'thread/event',
        params: {
          thread: 'thread-1',
          event: event(1, 'message.added', { role: 'user', parts: [{ type: 'text', text: 'hi' }] }),
        },
      },
      {
        jsonrpc: '2.0',
        method: 'turn/live',
        params: { thread: 'thread-1', update: { kind: 'text', text: 'Hel' } },
      },
      {
        jsonrpc: '2.0',
        method: 'turn/live',
        params: { thread: 'other', update: { kind: 'text', text: 'X' } },
      },
      {
        jsonrpc: '2.0',
        method: 'thread/event',
        params: {
          thread: 'thread-1',
          event: event(2, 'message.added', {
            role: 'assistant',
            parts: [{ type: 'text', text: 'Hello' }],
          }),
        },
      },
    ]);
    const service = make(fake);
    await service.open('thread-1');
    const sending = service.send('hi');
    expect(service.busy()).toBe(true);
    await sending;
    expect(service.busy()).toBe(false);
    expect(service.hasReply()).toBe(false);
    expect(service.items()).toMatchObject([
      { from: 'person', text: 'hi' },
      { from: 'model', text: 'Hello' },
    ]);
    expect(fake.paramsOf('turn/start')).toEqual([{ thread: 'thread-1', prompt: 'hi' }]);
  });

  it('decides, answers and stops, then catches up on what was recorded', async () => {
    const fake = new FakeSession().answer('thread/events', { events: [] });
    const service = make(fake);
    await service.open('thread-1');
    await service.decide('call_1', 'allow_once');
    await service.answer('call_2', ['yes']);
    await service.stop();
    expect(fake.paramsOf('approval/decide')).toEqual([
      { thread: 'thread-1', approval: 'call_1', decision: 'allow_once' },
    ]);
    expect(fake.paramsOf('question/answer')).toEqual([
      { thread: 'thread-1', question: 'call_2', answers: ['yes'] },
    ]);
    expect(fake.paramsOf('turn/cancel')).toEqual([{ thread: 'thread-1' }]);
    expect(fake.paramsOf('thread/events').length).toBeGreaterThan(1);
  });
});
