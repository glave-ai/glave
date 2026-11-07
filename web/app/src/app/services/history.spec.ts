import { TestBed } from '@angular/core/testing';
import type { RawEvent } from '@glave/client';

import { FakeSession, provideFakeSession } from '../testing/fake-session';
import { event } from '../testing/fixtures';
import { History } from './history';

function make(fake: FakeSession): History {
  TestBed.configureTestingModule({ providers: [...provideFakeSession(fake)] });
  return TestBed.runInInjectionContext(() => new History());
}

const message = (seq: number, text: string): RawEvent =>
  event(seq, 'message.added', { role: 'user', parts: [{ type: 'text', text }] });

const seqs = (history: History): readonly number[] => history.events().map((each) => each.seq);

describe('History', () => {
  it('opens a thread at its latest turns, and reads those before them when asked, once each', async () => {
    const fake = new FakeSession().answer('thread/turns', (params) =>
      (params as { before?: number }).before === undefined
        ? { events: [event(3, 'turn.started', {}), message(4, 'latest')], earlier: 3 }
        : { events: [event(1, 'turn.started', {}), message(2, 'first')] },
    );
    const history = make(fake);
    history.reset('thread-1');
    expect(history.opening()).toBe(true);
    await history.latest('thread-1');
    expect(history.opening()).toBe(false);
    expect(seqs(history)).toEqual([3, 4]);
    // What a fact of the thread is read again on: where its latest turns began, as anything
    // may have happened before them.
    expect(history.planning()).toBe(3);
    await history.earlier();
    expect(seqs(history)).toEqual([1, 2, 3, 4]);
    // The server said nothing came before: nothing more is asked.
    await history.earlier();
    expect(fake.paramsOf('thread/turns')).toEqual([
      { thread: 'thread-1' },
      { thread: 'thread-1', before: 3 },
    ]);
    expect(fake.paramsOf('thread/events')).toEqual([]);
  });

  it('reads every page of what was recorded since, and adds each event once', async () => {
    const fake = new FakeSession()
      .answer('thread/turns', { events: [message(1, 'a')] })
      .answer('thread/events', (params) =>
        (params as { after?: number }).after === 1
          ? { events: [message(2, 'b')], next: 2 }
          : { events: [message(3, 'c')] },
      );
    const history = make(fake);
    history.reset('thread-1');
    await history.latest('thread-1');
    expect(history.planning()).toBeNull();
    await history.after('thread-1', 1);
    expect(seqs(history)).toEqual([1, 2, 3]);
    expect(history.add(message(3, 'c'))).toBe(false);
    expect(history.add(message(4, 'd'))).toBe(true);
    expect(fake.paramsOf('thread/events')).toEqual([
      { thread: 'thread-1', after: 1 },
      { thread: 'thread-1', after: 2 },
    ]);
  });

  it('keeps nothing read for a thread it no longer holds', async () => {
    const fake = new FakeSession().answer('thread/turns', { events: [message(1, 'a')] });
    const history = make(fake);
    history.reset('thread-1');
    const reading = history.latest('thread-1');
    history.reset('thread-2');
    await reading;
    expect(seqs(history)).toEqual([]);
    expect(history.opening()).toBe(true);
  });
});
