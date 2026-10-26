import type { RawEvent } from '@glave/client';

import { event } from '../testing/fixtures';
import { askedTurns, threadTurns, turnRuns } from './turns';

function at(seq: number, type: string, data: Record<string, unknown>, time: string): RawEvent {
  return { ...event(seq, type, data), at: time };
}

const ASK = (seq: number, text: string): RawEvent =>
  event(seq, 'message.added', { role: 'user', parts: [{ type: 'text', text }] });
const SAY = (seq: number, text: string): RawEvent =>
  event(seq, 'message.added', { role: 'assistant', parts: [{ type: 'text', text }] });
const COST = (seq: number, micros: number, currency = 'USD'): RawEvent =>
  event(seq, 'model.responded', {
    model: { provider: 'p', name: 'm' },
    cost: { amount: { micros, currency }, price_list: 'x' },
  });

describe('turns', () => {
  it('splits a thread into turns at each of the person’s messages', () => {
    const turns = threadTurns([
      event(1, 'thread.started', {}),
      at(2, 'turn.started', {}, '2026-10-11T10:00:00.000Z'),
      ASK(3, 'Fix it'),
      event(4, 'tool.call.requested', {
        call_id: 'c1',
        tool: 'write',
        arguments: { json: { path: 'a.rs', content: 'one\ntwo\n' } },
      }),
      event(5, 'tool.call.completed', { call_id: 'c1', is_error: false, output: [] }),
      SAY(6, 'Done'),
      COST(7, 1500),
      at(8, 'turn.ended', { outcome: 'completed' }, '2026-10-11T10:04:02.000Z'),
      at(9, 'turn.started', {}, '2026-10-11T10:05:00.000Z'),
      ASK(10, 'Thanks'),
      at(11, 'message.added', { role: 'assistant', parts: [] }, '2026-10-11T10:05:12.000Z'),
    ]);
    expect(turns.map((turn) => turn.items.map((item) => item.kind))).toEqual([
      ['message', 'call', 'message'],
      ['message', 'message'],
    ]);
    const [first, second] = turns;
    expect(first).toMatchObject({
      took: 242_000,
      started: '2026-10-11T10:00:00.000Z',
      outcome: 'completed',
      cost: { micros: 1500, currency: 'USD' },
    });
    expect(first?.files.map((file) => [file.path, file.added])).toEqual([['a.rs', 2]]);
    // Still going: no end, its time so far, no cost said.
    expect(second).toMatchObject({
      took: 12_000,
      started: '2026-10-11T10:05:00.000Z',
      outcome: null,
      cost: null,
      files: [],
    });
  });

  it('keeps a turn whole when its start was recorded after the person’s message', () => {
    const turns = threadTurns([ASK(1, 'Hi'), event(2, 'turn.started', {}), SAY(3, 'Hello')]);
    expect(turns).toHaveLength(1);
  });

  it('starts a turn at a turn.started that follows work, with no message of the person’s', () => {
    const turns = threadTurns([
      ASK(1, 'Plan it'),
      SAY(2, 'Planned'),
      event(3, 'turn.started', {}),
      SAY(4, 'Carrying on with the goal'),
    ]);
    expect(turns.map((turn) => turn.items.length)).toEqual([2, 1]);
  });

  it('starts the turn that builds a plan at the person’s Build it, glave’s prompt in it', () => {
    const turns = threadTurns([
      event(1, 'turn.started', {}),
      ASK(2, 'Add a flag'),
      SAY(3, 'Here is the plan'),
      event(4, 'turn.ended', { outcome: 'completed' }),
      event(5, 'plan.approved', {}),
      event(6, 'permissions.chosen', { permissions: 'auto' }),
      event(7, 'turn.started', {}),
      event(8, 'message.added', {
        role: 'user',
        parts: [{ type: 'text', text: 'The person approved this plan. Build it now.' }],
        author: { kind: 'glave' },
      }),
      SAY(9, 'Built'),
    ]);
    expect(
      turns.map((turn) =>
        turn.items.map((item) => (item.kind === 'message' ? item.built === true : item.kind)),
      ),
    ).toEqual([
      [false, false],
      [true, false],
    ]);
  });

  it('says where a turn’s model calls ran, once each, as their records name them', () => {
    const ran = (data: Readonly<Record<string, unknown>>, seq: number): RawEvent =>
      event(seq, 'model.responded', data);
    const routes = (events: readonly RawEvent[]): unknown =>
      threadTurns([ASK(1, 'x'), ...events])[0]?.routes;
    expect(
      routes([
        ran({ service: 'OpenRouter', upstream: 'Anthropic' }, 2),
        ran({ service: 'OpenRouter', upstream: 'Anthropic' }, 3),
        ran({ service: 'Anthropic' }, 4),
        ran({}, 5),
      ]),
    ).toEqual(['OpenRouter (Anthropic)', 'Anthropic']);
    expect(routes([])).toEqual([]);
  });

  it('says what a turn cost only when every response said, in one currency', () => {
    const cost = (events: readonly RawEvent[]): unknown =>
      threadTurns([ASK(1, 'x'), ...events])[0]?.cost;
    expect(cost([COST(2, 100), COST(3, 250)])).toEqual({ micros: 350, currency: 'USD' });
    expect(cost([COST(2, 100), event(3, 'model.responded', {})])).toBeNull();
    expect(cost([COST(2, 100), COST(3, 100, 'EUR')])).toBeNull();
    expect(cost([])).toBeNull();
  });

  it('makes no turn of events that put nothing in the conversation', () => {
    expect(threadTurns([event(1, 'thread.started', {}), event(2, 'model.chosen', {})])).toEqual([]);
  });

  it('keys a turn the view asked for by its asking, from before it is recorded until after', () => {
    const before = threadTurns([ASK(1, 'Hi'), SAY(2, 'Hello')]);
    // Asked after event 2: the person's message alone, with glave working under it.
    const asking = askedTurns(before, [2], 'Fix the flaky test');
    expect(asking.map((turn) => turn.key)).toEqual(['turn-1', 'turn-asked-2']);
    expect(asking.at(-1)?.items).toEqual([
      expect.objectContaining({ kind: 'message', from: 'person', text: 'Fix the flaky test' }),
    ]);
    expect(asking.at(-1)?.outcome).toBeNull();
    // A choice and the turn's start recorded, not yet the message: still the turn asked for.
    const started = [
      ...[ASK(1, 'Hi'), SAY(2, 'Hello')],
      event(3, 'model.chosen', { model: 'hope-12b' }),
      event(4, 'turn.started', {}),
    ];
    const keys = (events: readonly RawEvent[], prompt: string | null): readonly string[] =>
      askedTurns(threadTurns(events), [2], prompt).map((turn) => turn.key);
    expect(keys(started, 'Fix the flaky test')).toEqual(['turn-1', 'turn-asked-2']);
    // Recorded, and on after it: the same key.
    const recorded = [...started, ASK(5, 'Fix the flaky test'), SAY(6, 'Fixed')];
    expect(keys(recorded, null)).toEqual(['turn-1', 'turn-asked-2']);
    // Asked for again: each keeps its own.
    expect(
      askedTurns(threadTurns([...recorded, ASK(7, 'Thanks')]), [2, 6], null).map(
        (turn) => turn.key,
      ),
    ).toEqual(['turn-1', 'turn-asked-2', 'turn-asked-6']);
    // A turn the view did not ask for keeps the key its events give it.
    expect(keys([ASK(1, 'Hi')], null)).toEqual(['turn-1']);
  });

  it('says a turn runs from its start until it ends', () => {
    expect(turnRuns([])).toBe(false);
    const started = [event(1, 'thread.started', {}), event(2, 'turn.started', {}), ASK(3, 'Go')];
    expect(turnRuns(started)).toBe(true);
    expect(turnRuns([...started, event(4, 'turn.ended', { outcome: 'completed' })])).toBe(false);
    expect(turnRuns([...started, event(4, 'turn.ended', {}), event(5, 'thread.renamed', {})])).toBe(
      false,
    );
  });
});
