import { TestBed } from '@angular/core/testing';

import { ApiError, ErrorCode, type Notification, type RawEvent, type Update } from '@glave/client';

import { FakeSession, FakeTurn, provideFakeSession } from '../testing/fake-session';
import { event, thread } from '../testing/fixtures';
import { ThreadService } from './thread.service';

function make(fake: FakeSession): ThreadService {
  TestBed.configureTestingModule({ providers: [...provideFakeSession(fake), ThreadService] });
  return TestBed.inject(ThreadService);
}

/** A stored event of thread-1, as a turn sends it. */
function recorded(stored: RawEvent): Notification {
  return { jsonrpc: '2.0', method: 'thread/event', params: { thread: 'thread-1', event: stored } };
}

/** Progress in thread-1, as a turn sends it. */
function live(update: Update): Notification {
  return { jsonrpc: '2.0', method: 'turn/live', params: { thread: 'thread-1', update } };
}

/** The person's message, recorded at `seq`. */
function said(seq: number, text: string): Notification {
  return recorded(event(seq, 'message.added', { role: 'user', parts: [{ type: 'text', text }] }));
}

/** Lets every promise the app is waiting on go on. */
function settled(): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, 0));
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

  it('queues what is sent while a turn runs, until the model reads it', async () => {
    const first = new FakeTurn();
    const fake = new FakeSession()
      .answer('thread/events', { events: [] })
      .turn(first)
      .turn({ queued: { id: 'event-4' } });
    const service = make(fake);
    await service.open('thread-1');
    const sending = service.send('set the port to 3000');
    first.send(recorded(event(1, 'turn.started', {})), said(2, 'set the port to 3000'));
    await settled();
    await service.send('actually, 4000', 'hope-12b', 'ask');
    // The model and mode wait for the next turn: a message to a running turn chooses neither.
    expect(fake.paramsOf('turn/start')).toEqual([
      { thread: 'thread-1', prompt: 'set the port to 3000' },
      { thread: 'thread-1', prompt: 'actually, 4000' },
    ]);
    expect(service.busy()).toBe(true);
    expect(service.queued().map((message) => message.text)).toEqual(['actually, 4000']);
    first.send(recorded(event(3, 'tool.call.completed', { call_id: 'call_1' })));
    await settled();
    expect(service.queued()).toHaveLength(1);
    // Read: its event, under the id the server answered with, is in the thread.
    first.send(said(4, 'actually, 4000'));
    await settled();
    expect(service.queued()).toEqual([]);
    const items = service.turns().flatMap((turn) => turn.items);
    expect(items.at(-1)).toMatchObject({ from: 'person', text: 'actually, 4000', steered: true });
    first.end();
    await sending;
    expect(service.busy()).toBe(false);
    expect(service.composer()).toBe('');
  });

  it('puts a message a hook stopped back in the box, saying why', async () => {
    const first = new FakeTurn();
    const fake = new FakeSession()
      .answer('thread/events', { events: [] })
      .turn(first)
      .turn({ queued: { id: 'event-9' } });
    const service = make(fake);
    await service.open('thread-1');
    const sending = service.send('start');
    await settled();
    await service.send('my secret is 42');
    first.send(live({ kind: 'unread', id: 'event-9', reason: 'that looks like a secret' }));
    await settled();
    expect(service.queued()).toEqual([]);
    expect(service.composer()).toBe('my secret is 42');
    expect(service.error()).toBe('that looks like a secret');
    first.end();
    await sending;
    expect(fake.paramsOf('turn/start')).toHaveLength(2);
  });

  it('puts a message the server refused back in the box at once, saying why', async () => {
    const first = new FakeTurn();
    const full = new ApiError({
      code: ErrorCode.QUEUE_FULL,
      message: '20 messages are already waiting for the model to read them',
    });
    const fake = new FakeSession().answer('thread/events', { events: [] }).turn(first).turn(full);
    const service = make(fake);
    await service.open('thread-1');
    const sending = service.send('go');
    await settled();
    await service.send('one too many');
    expect(service.queued()).toEqual([]);
    expect(service.composer()).toBe('one too many');
    expect(service.error()).toContain('20 messages are already waiting');
    first.end();
    await sending;
    expect(service.composer()).toBe('one too many');
  });

  it('reads the lists again, rather than showing an error, when the project’s folder went', async () => {
    const gone = new ApiError({
      code: ErrorCode.NO_FOLDER,
      message: "This project's folder isn't there anymore.",
    });
    const fake = new FakeSession()
      .answer('thread/events', { events: [] })
      .answer('space/list', { spaces: [] })
      .turn(gone);
    const service = make(fake);
    await service.open('thread-1');
    await service.send('go');
    await settled();
    expect(service.error()).toBeNull();
    expect(fake.paramsOf('space/list')).toHaveLength(1);
  });

  it('sends what was waiting, together, once the person stops the turn', async () => {
    const first = new FakeTurn();
    const fake = new FakeSession()
      .answer('thread/events', { events: [] })
      .turn(first)
      .turn({ queued: { id: 'event-7' } })
      .turn({ queued: { id: 'event-8' } });
    const service = make(fake);
    await service.open('thread-1');
    const sending = service.send('go');
    await settled();
    await service.send('and this');
    await service.send('and that');
    const stopping = service.stop();
    first.send(live({ kind: 'unread', id: 'event-7' }), live({ kind: 'unread', id: 'event-8' }));
    first.end({ turn: { outcome: 'cancelled', model_calls: 1, tool_calls: 0, tokens: {} } });
    await stopping;
    await sending;
    await settled();
    expect(fake.paramsOf('turn/cancel')).toEqual([{ thread: 'thread-1' }]);
    expect(fake.paramsOf('turn/start').at(-1)).toEqual({
      thread: 'thread-1',
      prompt: 'and this\n\nand that',
    });
    expect(service.composer()).toBe('');
  });

  it('puts what the model never read back in the box when the turn failed', async () => {
    const first = new FakeTurn();
    const fake = new FakeSession()
      .answer('thread/events', { events: [] })
      .turn(first)
      .turn({ queued: { id: 'event-7' } });
    const service = make(fake);
    await service.open('thread-1');
    const sending = service.send('go');
    await settled();
    service.composer.set('half written');
    await service.send('and this');
    first.send(live({ kind: 'unread', id: 'event-7' }));
    first.end({ turn: { outcome: 'failed', model_calls: 1, tool_calls: 0, tokens: {} } });
    await sending;
    expect(service.composer()).toBe('and this\n\nhalf written');
    expect(fake.paramsOf('turn/start')).toHaveLength(2);
  });

  it('follows a message that started a turn of its own: the one it was sent to had ended', async () => {
    const [first, own] = [new FakeTurn(), new FakeTurn()];
    const fake = new FakeSession().answer('thread/events', { events: [] }).turn(first).turn(own);
    const service = make(fake);
    await service.open('thread-1');
    const sending = service.send('one');
    await settled();
    const second = service.send('two');
    first.end();
    await sending;
    expect(service.busy()).toBe(true);
    own.send(recorded(event(5, 'turn.started', {})), said(6, 'two'));
    await settled();
    expect(service.queued()).toEqual([]);
    own.end();
    await second;
    expect(service.busy()).toBe(false);
    expect(service.composer()).toBe('');
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
