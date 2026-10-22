import { TestBed } from '@angular/core/testing';

import { ApiError, ErrorCode, type Notification, type RawEvent, type Update } from '@glave/client';

import { FakeSession, FakeTurn, provideFakeSession } from '../testing/fake-session';
import { event, thread } from '../testing/fixtures';
import { FirstTurnService } from './first-turn.service';
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
    // With a key of its own, so whatever repeats the request, the server starts one turn.
    const key = expect.stringMatching(/^[0-9a-f]{32}$/u) as unknown;
    expect(fake.paramsOf('turn/start')).toEqual([
      { thread: 'thread-1', prompt: 'hi', _meta: { idempotency_key: key } },
    ]);
  });

  it('shows the first turn the new chat sent until its record arrives, a message waiting behind it', async () => {
    const watch = new FakeTurn();
    const fake = new FakeSession()
      .answer('thread/events', { events: [] })
      .turn(new FakeTurn())
      .turn({ queued: { id: 'event-9' } })
      .watch(watch);
    TestBed.configureTestingModule({ providers: [...provideFakeSession(fake), ThreadService] });
    const firstTurns = TestBed.inject(FirstTurnService);
    firstTurns.start({ thread: 'thread-1', prompt: 'Plan' });
    const service = TestBed.inject(ThreadService);
    await service.open('thread-1');
    expect([service.starting(), service.busy()]).toEqual(['Plan', true]);
    // Sent while the first turn is prepared, it waits for the model rather than coming back.
    await service.send('and the tests');
    expect(service.queued().map((message) => message.text)).toEqual(['and the tests']);
    expect(service.composer()).toBe('');
    watch.send(said(1, 'Plan'));
    await settled();
    expect([service.starting(), firstTurns.of('thread-1')]).toEqual([null, null]);
  });

  it('lets go of a first turn whose record is there as the thread opens', async () => {
    const asked = event(1, 'message.added', {
      role: 'user',
      parts: [{ type: 'text', text: 'Plan' }],
    });
    const fake = new FakeSession()
      .answer('thread/events', { events: [asked] })
      .turn(new FakeTurn());
    TestBed.configureTestingModule({ providers: [...provideFakeSession(fake), ThreadService] });
    const firstTurns = TestBed.inject(FirstTurnService);
    firstTurns.start({ thread: 'thread-1', prompt: 'Plan' });
    await TestBed.inject(ThreadService).open('thread-1');
    expect(firstTurns.of('thread-1')).toBeNull();
  });

  it('puts a first turn the server refused back in the box, saying why', async () => {
    const full = new ApiError({ code: ErrorCode.QUEUE_FULL, message: 'Too many are waiting' });
    const fake = new FakeSession().answer('thread/events', { events: [] }).turn(full);
    TestBed.configureTestingModule({ providers: [...provideFakeSession(fake), ThreadService] });
    const firstTurns = TestBed.inject(FirstTurnService);
    firstTurns.start({ thread: 'thread-1', prompt: 'Plan' });
    const service = TestBed.inject(ThreadService);
    await service.open('thread-1');
    await settled();
    TestBed.tick();
    expect([service.composer(), service.error(), firstTurns.of('thread-1')]).toEqual([
      'Plan',
      'Too many are waiting',
      null,
    ]);
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

  it('says the step a running turn takes in the server’s words, from its start to its end', async () => {
    const turn = new FakeTurn();
    const fake = new FakeSession().answer('thread/events', { events: [] }).turn(turn);
    const service = make(fake);
    await service.open('thread-1');
    const sending = service.send('run the tests');
    turn.send(recorded(event(1, 'turn.started', {})), said(2, 'run the tests'));
    await settled();
    expect(service.step()).toBeNull();
    turn.send(live({ kind: 'tool_started', summary: 'bash: cargo test' }));
    await settled();
    expect(service.step()).toBe('bash: cargo test');
    turn.send(live({ kind: 'tool_finished', summary: 'bash: cargo test', is_error: false }));
    await settled();
    expect(service.step()).toBeNull();
    // A turn that ends takes what it showed live with it.
    turn.send(live({ kind: 'tool_started', summary: 'bash: cargo build' }));
    await settled();
    expect(service.step()).toBe('bash: cargo build');
    turn.send(recorded(event(3, 'turn.ended', { outcome: 'cancelled' })));
    turn.end();
    await sending;
    expect(service.step()).toBeNull();
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
    expect(fake.paramsOf('turn/start')).toMatchObject([
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
    first.send(recorded(event(5, 'turn.ended', { outcome: 'completed' })));
    first.end();
    await sending;
    expect(service.busy()).toBe(false);
    expect(service.composer()).toBe('');
  });

  it('shows a memory call’s result as the server reads it for the person, live too', async () => {
    const turn = new FakeTurn();
    const words = 'Remembered [m1] (personal) Ada prefers tea.';
    const kept = event(4, 'tool.call.completed', {
      call_id: 'call_1',
      is_error: false,
      output: [{ type: 'text', text: 'Remembered [m1] (personal).' }],
      structured: { memory: 'm1', version: 1, shown: { reply: 'remembered' } },
    });
    const fake = new FakeSession()
      .answer('thread/events', (params) =>
        (params as { after?: number }).after === 3
          ? { events: [kept], results: { call_1: words } }
          : { events: [] },
      )
      .turn(turn);
    const service = make(fake);
    await service.open('thread-1');
    const sending = service.send('remember that I like tea');
    turn.send(
      recorded(event(1, 'turn.started', {})),
      said(2, 'remember that I like tea'),
      recorded(
        event(3, 'tool.call.requested', {
          call_id: 'call_1',
          tool: 'mcp__memory__remember',
          arguments: { json: { text: 'Ada prefers tea', scope: 'personal' } },
        }),
      ),
      recorded(kept),
    );
    await settled();
    const call = service
      .turns()
      .flatMap((one) => one.items)
      .find((item) => item.kind === 'call');
    expect(call).toMatchObject({ output: words });
    expect(fake.paramsOf('thread/events')).toContainEqual({
      thread: 'thread-1',
      after: 3,
      limit: 1,
    });
    turn.end();
    await sending;
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

  it('shows what the person sent at once, until the server records their message', async () => {
    const turn = new FakeTurn();
    const fake = new FakeSession().answer('thread/events', { events: [] }).turn(turn);
    const service = make(fake);
    await service.open('thread-1');
    const sending = service.send('go');
    expect(service.starting()).toBe('go');
    turn.send(recorded(event(1, 'turn.started', {})));
    await settled();
    expect(service.starting()).toBe('go');
    turn.send(said(2, 'go'));
    await settled();
    expect(service.starting()).toBeNull();
    turn.end();
    await sending;
    expect(service.starting()).toBeNull();
  });

  it('puts a turn the server refused to start back in the box, saying why', async () => {
    const busy = new ApiError({ code: ErrorCode.QUEUE_FULL, message: 'The server is stopping' });
    const fake = new FakeSession().answer('thread/events', { events: [] }).turn(busy);
    const service = make(fake);
    await service.open('thread-1');
    service.composer.set('and then');
    await service.send('go');
    expect(service.starting()).toBeNull();
    expect(service.composer()).toBe('go\n\nand then');
    expect(service.error()).toContain('The server is stopping');
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
      .answer('project/list', { projects: [] })
      .turn(gone);
    const service = make(fake);
    await service.open('thread-1');
    await service.send('go');
    await settled();
    expect(service.error()).toBeNull();
    expect(fake.paramsOf('project/list')).toHaveLength(1);
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
    expect(fake.paramsOf('turn/start').at(-1)).toMatchObject({
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
    own.send(recorded(event(7, 'turn.ended', { outcome: 'completed' })));
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
    expect(fake.paramsOf('turn/start')).toMatchObject([
      { thread: 'thread-1', prompt: 'hi', model: 'hope-12b' },
    ]);
    expect(service.chosen()).toBe('hope-12b');
  });

  it('renames the thread, which the lists hear of from the server, or says why it could not', async () => {
    const fake = new FakeSession()
      .answer('thread/events', { events: [] })
      .answer('project/list', { projects: [] });
    const service = make(fake);
    await service.rename('Nothing open');
    expect(fake.paramsOf('thread/rename')).toEqual([]);
    await service.open('thread-1');
    await service.rename('Launch plan');
    expect(fake.paramsOf('thread/rename')).toEqual([{ thread: 'thread-1', title: 'Launch plan' }]);
    expect(fake.paramsOf('project/list')).toHaveLength(0);
    fake.answer(
      'thread/rename',
      new ApiError({ code: ErrorCode.TURN_RUNNING, message: 'a turn is running' }),
    );
    await service.rename('Again');
    expect(service.error()).toBe('a turn is running');
  });

  it('follows the thread live once read: a turn another tab started, its progress and its approval', async () => {
    const watch = new FakeTurn();
    const fake = new FakeSession()
      .answer('thread/events', { events: [event(1, 'thread.started', {})] })
      .watch(watch);
    const service = make(fake);
    await service.open('thread-1');
    expect(fake.paramsOf('thread/watch')).toEqual([{ thread: 'thread-1', after: 1 }]);
    expect(service.busy()).toBe(false);
    watch.send(
      recorded(event(2, 'turn.started', {})),
      said(3, 'touch a file'),
      live({ kind: 'text', text: 'Let me' }),
    );
    await settled();
    expect(service.busy()).toBe(true);
    expect(service.reply().text).toBe('Let me');
    watch.send(
      recorded(
        event(4, 'approval.requested', {
          call_id: 'call_1',
          tool: 'bash',
          summary: 'touch a.txt',
        }),
      ),
    );
    await settled();
    const items = service.turns().flatMap((turn) => turn.items);
    expect(items.map((item) => item.kind)).toEqual(['message', 'approval']);
    watch.send(recorded(event(5, 'turn.ended', { outcome: 'cancelled' })));
    await settled();
    expect(service.busy()).toBe(false);
    expect(service.reply().text).toBe('');
  });

  it('rests its watch while it follows a turn of its own, then watches from where it ended', async () => {
    const [before, after] = [new FakeTurn(), new FakeTurn()];
    const turn = new FakeTurn();
    const fake = new FakeSession()
      .answer('thread/events', { events: [event(1, 'thread.started', {})] })
      .watch(before)
      .watch(after)
      .turn(turn);
    const service = make(fake);
    await service.open('thread-1');
    const sending = service.send('hi');
    turn.send(
      recorded(event(2, 'turn.started', {})),
      said(3, 'hi'),
      live({ kind: 'text', text: 'Hel' }),
    );
    await settled();
    // The watch rested: what it would have said is not taken.
    before.send(live({ kind: 'text', text: 'Hel' }));
    await settled();
    expect(service.reply().text).toBe('Hel');
    turn.send(recorded(event(4, 'turn.ended', { outcome: 'completed' })));
    turn.end();
    await sending;
    await settled();
    expect(fake.paramsOf('thread/watch')).toEqual([
      { thread: 'thread-1', after: 1 },
      { thread: 'thread-1', after: 4 },
    ]);
    expect(service.reply().text).toBe('');
  });

  it('shows a turn the log leaves open as running only when the server says it runs', async () => {
    const open = [event(1, 'thread.started', {}), event(2, 'turn.started', {})];
    const stopped = make(new FakeSession().answer('thread/events', { events: open }));
    await stopped.open('thread-1');
    expect(stopped.busy()).toBe(false);
    TestBed.resetTestingModule();
    const waiting = make(
      new FakeSession()
        .answer('thread/events', { events: open })
        .answer('thread/get', { thread: thread('thread-1', null, { status: 'waiting' }) }),
    );
    await waiting.open('thread-1');
    expect(waiting.busy()).toBe(true);
  });

  it('keeps waiting a message the server queued to a turn that had started meanwhile', async () => {
    const fake = new FakeSession()
      .answer('thread/events', { events: [] })
      .turn({ queued: { id: 'event-5' } });
    const service = make(fake);
    await service.open('thread-1');
    await service.send('one more thing');
    expect(service.queued().map((message) => message.text)).toEqual(['one more thing']);
    expect(service.starting()).toBeNull();
  });

  it('gives back what waited for a turn it lost sight of, once no turn runs', async () => {
    vi.useFakeTimers();
    try {
      const cut = new FakeTurn();
      let [read, gets] = [0, 0];
      const fake = new FakeSession()
        .answer('thread/events', () => {
          read += 1;
          return read === 1
            ? { events: [event(1, 'turn.started', {})] }
            : { events: [event(2, 'turn.ended', { outcome: 'cancelled' })] };
        })
        .answer('thread/get', () => {
          gets += 1;
          return { thread: thread('thread-1', null, { status: gets === 1 ? 'running' : 'idle' }) };
        })
        .watch(cut)
        .turn({ queued: { id: 'event-9' } });
      const service = make(fake);
      await service.open('thread-1');
      expect(service.busy()).toBe(true);
      await service.send('and this');
      expect(service.queued().map((message) => message.text)).toEqual(['and this']);
      // Cut off while the turn ended and gave the message back, unseen.
      cut.end({ ended: 'unwatched' });
      await vi.advanceTimersByTimeAsync(1_000);
      expect(service.queued()).toEqual([]);
      expect(service.composer()).toBe('and this');
      expect(service.busy()).toBe(false);
    } finally {
      vi.useRealTimers();
    }
  });

  it('keeps a message queued to a turn another tab follows until that turn reads it', async () => {
    const watch = new FakeTurn();
    const fake = new FakeSession()
      .answer('thread/events', { events: [] })
      .watch(watch)
      .turn({ queued: { id: 'event-9' } });
    const service = make(fake);
    await service.open('thread-1');
    watch.send(recorded(event(1, 'turn.started', {})), said(2, 'go'));
    await settled();
    await service.send('and then this');
    expect(service.queued().map((message) => message.text)).toEqual(['and then this']);
    expect(service.composer()).toBe('');
    watch.send({
      jsonrpc: '2.0',
      method: 'thread/event',
      params: {
        thread: 'thread-1',
        event: {
          ...event(3, 'message.added', {
            role: 'user',
            parts: [{ type: 'text', text: 'and then this' }],
          }),
          id: 'event-9',
        },
      },
    });
    await settled();
    expect(service.queued()).toEqual([]);
  });

  it('comes back from where it was once its watch is cut off, and not for a deleted thread', async () => {
    vi.useFakeTimers();
    try {
      const cut = new FakeTurn();
      const again = new FakeTurn();
      const fake = new FakeSession()
        .answer('thread/events', { events: [event(1, 'thread.started', {})] })
        .watch(cut)
        .watch(again);
      const service = make(fake);
      await service.open('thread-1');
      cut.send(recorded(event(2, 'thread.renamed', { title: 'Garden' })));
      cut.end({ ended: 'unwatched' });
      await vi.advanceTimersByTimeAsync(1_000);
      expect(fake.paramsOf('thread/watch')).toEqual([
        { thread: 'thread-1', after: 1 },
        { thread: 'thread-1', after: 2 },
      ]);
      again.end({ ended: 'deleted' });
      await vi.advanceTimersByTimeAsync(60_000);
      expect(fake.paramsOf('thread/watch').length).toBe(2);
      expect(service.error()).toBeNull();
    } finally {
      vi.useRealTimers();
    }
  });

  it('lets go of its watch while the tab is hidden, and comes back from where it was', async () => {
    const fake = new FakeSession().answer('thread/events', {
      events: [event(1, 'thread.started', {})],
    });
    const service = make(fake);
    await service.open('thread-1');
    const shown = (state: DocumentVisibilityState): void => {
      Object.defineProperty(document, 'visibilityState', { value: state, configurable: true });
      document.dispatchEvent(new Event('visibilitychange'));
    };
    try {
      shown('hidden');
      await settled();
      shown('visible');
      await settled();
      expect(fake.paramsOf('thread/watch')).toEqual([
        { thread: 'thread-1', after: 1 },
        { thread: 'thread-1', after: 1 },
      ]);
    } finally {
      Reflect.deleteProperty(document, 'visibilityState');
    }
  });

  it('asks the server again, coming back, whether the turn it saw start still runs', async () => {
    vi.useFakeTimers();
    try {
      const cut = new FakeTurn();
      const fake = new FakeSession()
        .answer('thread/events', { events: [event(1, 'thread.started', {})] })
        .watch(cut);
      const service = make(fake);
      await service.open('thread-1');
      cut.send(recorded(event(2, 'turn.started', {})));
      await vi.advanceTimersByTimeAsync(0);
      expect(service.busy()).toBe(true);
      // glave stopped mid-turn: the log leaves it open, and the server says no turn runs.
      cut.end({ ended: 'unwatched' });
      await vi.advanceTimersByTimeAsync(1_000);
      expect(service.busy()).toBe(false);
    } finally {
      vi.useRealTimers();
    }
  });

  it('reads from the log an event that came between two streams', async () => {
    const watch = new FakeTurn();
    const fake = new FakeSession()
      .answer('thread/events', (params) =>
        (params as { after?: number }).after === undefined
          ? { events: [event(1, 'thread.started', {})] }
          : { events: [event(2, 'thread.renamed', { title: 'Garden' })] },
      )
      .watch(watch);
    const service = make(fake);
    await service.open('thread-1');
    watch.send(recorded(event(3, 'turn.started', {})));
    await settled();
    expect(fake.paramsOf('thread/events')).toEqual([
      { thread: 'thread-1' },
      { thread: 'thread-1', after: 1 },
    ]);
  });

  it('takes a turn’s end read after the server said it runs, coming back', async () => {
    vi.useFakeTimers();
    try {
      const cut = new FakeTurn();
      let read = 0;
      const fake = new FakeSession()
        .answer('thread/events', () => {
          read += 1;
          return read === 1
            ? { events: [event(1, 'turn.started', {})] }
            : { events: [event(2, 'turn.ended', { outcome: 'completed' })] };
        })
        .answer('thread/get', { thread: thread('thread-1', null, { status: 'running' }) })
        .watch(cut);
      const service = make(fake);
      await service.open('thread-1');
      expect(service.busy()).toBe(true);
      // The turn ended while the server was asked: what was recorded after says so.
      cut.end({ ended: 'unwatched' });
      await vi.advanceTimersByTimeAsync(1_000);
      expect(service.busy()).toBe(false);
    } finally {
      vi.useRealTimers();
    }
  });
});
