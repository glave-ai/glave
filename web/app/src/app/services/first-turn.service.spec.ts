import { TestBed } from '@angular/core/testing';
import { ApiError, ErrorCode, type Notification, type TurnStartParams } from '@glave/client';

import { FakeSession, FakeTurn, provideFakeSession } from '../testing/fake-session';
import { event } from '../testing/fixtures';
import { FirstTurnService } from './first-turn.service';
import { ThreadService } from './thread.service';

function settled(): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, 0));
}

/** The person's message in thread-1, recorded at `seq`, as its watch sends it. */
function said(seq: number, text: string): Notification {
  const stored = event(seq, 'message.added', { role: 'user', parts: [{ type: 'text', text }] });
  return { jsonrpc: '2.0', method: 'thread/event', params: { thread: 'thread-1', event: stored } };
}

describe('FirstTurnService', () => {
  it('sends a first turn with a key, says why it did not go, and sends it again with that key', async () => {
    const refused = new ApiError({
      code: ErrorCode.QUEUE_FULL,
      message: 'Too many messages are waiting',
    });
    const cut = new FakeTurn();
    cut.fail(new TypeError('Failed to fetch'));
    const fake = new FakeSession().turn(cut).turn(refused);
    TestBed.configureTestingModule({ providers: [...provideFakeSession(fake)] });
    const service = TestBed.inject(FirstTurnService);
    service.start({ thread: 't1', prompt: 'Plan' });
    service.start({ thread: 't2', prompt: 'Tidy' });
    await settled();
    expect([service.of('t1')?.failed, service.of('t2')?.failed]).toEqual([
      'lost',
      { said: 'Too many messages are waiting' },
    ]);
    service.again('t1');
    await settled();
    expect(service.of('t1')?.failed).toBeNull();
    service.forget('t1');
    expect([service.of('t1'), service.of('t2')?.params.prompt]).toEqual([null, 'Tidy']);
    const keys = fake
      .paramsOf('turn/start')
      .map((params) => (params as TurnStartParams)._meta?.idempotency_key);
    expect(keys[0]).toMatch(/^[0-9a-f]{32}$/u);
    expect(keys[2]).toBe(keys[0]);
    expect(new Set(keys).size).toBe(2);
  });
});

describe('FirstTurnService with the thread’s page', () => {
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
});
