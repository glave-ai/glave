import { TestBed } from '@angular/core/testing';

import { ApiError, ErrorCode } from '@glave/client';

import { FakeSession, provideFakeSession } from '../testing/fake-session';
import { event, thread } from '../testing/fixtures';
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
    expect(service.turns().flatMap((turn) => turn.items.map((item) => item.kind))).toEqual([
      'message',
      'message',
    ]);
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
    expect(service.turns().map((turn) => turn.items)).toMatchObject([
      [
        { from: 'person', text: 'hi' },
        { from: 'model', text: 'Hello' },
      ],
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

  it('raises the project’s ask on opening, and answers it once', async () => {
    const ask = { ask: 'ask-1', items: [{ text: 'Runs a tool server', detail: 'npx @acme/mcp' }] };
    const fake = new FakeSession()
      .answer('thread/events', { events: [] })
      .answer('thread/get', (params) => {
        const id = (params as { thread: string }).thread;
        return { thread: thread(id, 'Fix it', id === 'thread-1' ? { trust: ask } : {}) };
      })
      .answer('trust/answer', { thread: thread('thread-1', 'Fix it') });
    const service = make(fake);
    await service.open('thread-1');
    expect(fake.paramsOf('thread/get')).toEqual([{ thread: 'thread-1' }]);
    expect(service.trust()).toEqual(ask);
    expect(service.trustAnswer()).toBeNull();
    await service.answerTrust('not_now');
    await service.answerTrust('trust');
    expect(fake.paramsOf('trust/answer')).toEqual([
      { thread: 'thread-1', ask: 'ask-1', answer: 'not_now' },
    ]);
    expect(service.trustAnswer()).toBe('not_now');
    expect(service.trust()).toEqual(ask);
    await service.open('thread-2');
    expect(service.trust()).toBeNull();
  });

  it('says why an answer was refused, and shows what the project asks now', async () => {
    const asked = (ask: string): unknown => ({
      thread: thread('thread-1', 'Fix it', {
        trust: { ask, items: [{ text: 'Runs a tool server' }] },
      }),
    });
    let reads = 0;
    const fake = new FakeSession()
      .answer('thread/events', { events: [] })
      .answer('thread/get', () => asked((reads += 1) === 1 ? 'ask-1' : 'ask-2'))
      .answer(
        'trust/answer',
        new ApiError({
          code: ErrorCode.NO_TRUST,
          message: 'this project changed since you were asked',
        }),
      );
    const service = make(fake);
    await service.open('thread-1');
    await service.answerTrust('trust');
    expect(service.error()).toContain('changed since you were asked');
    expect(service.trust()?.ask).toBe('ask-2');
    expect(service.trustAnswer()).toBeNull();
  });

  it('sends a turn to the model picked, and reads the thread’s choice', async () => {
    const fake = new FakeSession()
      .answer('thread/events', {
        events: [event(1, 'model.chosen', { model: 'gpt-6-luna' })],
      })
      .streams([
        {
          jsonrpc: '2.0',
          method: 'thread/event',
          params: { thread: 'thread-1', event: event(2, 'model.chosen', { model: 'hope-12b' }) },
        },
      ]);
    const service = make(fake);
    await service.open('thread-1');
    expect(service.chosen()).toBe('gpt-6-luna');
    await service.send('hi', 'hope-12b');
    expect(fake.paramsOf('turn/start')).toEqual([
      { thread: 'thread-1', prompt: 'hi', model: 'hope-12b' },
    ]);
    expect(service.chosen()).toBe('hope-12b');
  });

  it('renames the thread and reads the lists again, or says why it could not', async () => {
    const fake = new FakeSession()
      .answer('thread/events', { events: [] })
      .answer('space/list', { spaces: [] });
    const service = make(fake);
    await service.rename('Nothing open');
    expect(fake.paramsOf('thread/rename')).toEqual([]);
    await service.open('thread-1');
    await service.rename('Launch plan');
    expect(fake.paramsOf('thread/rename')).toEqual([{ thread: 'thread-1', title: 'Launch plan' }]);
    expect(fake.paramsOf('space/list')).toHaveLength(1);
    fake.answer(
      'thread/rename',
      new ApiError({ code: ErrorCode.TURN_RUNNING, message: 'a turn is running' }),
    );
    await service.rename('Again');
    expect(service.error()).toBe('a turn is running');
  });
});
