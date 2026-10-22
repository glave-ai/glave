import type { RawEvent } from '@glave/client';

import { event } from '../testing/fixtures';
import { startingTurn, threadTurns, turnRuns } from './turns';

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
    // Each keyed by its start, as the turn the person asked for was before it was recorded.
    expect(turns.map((turn) => turn.key)).toEqual(['turn-2', 'turn-9']);
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

  it('starts a turn the person just asked for with their message alone, not yet ended', () => {
    const starting = startingTurn('Fix the flaky test', 4);
    // Keyed as the turn will be once the server records its start next.
    expect(starting.key).toBe('turn-4');
    expect(starting.items).toEqual([
      expect.objectContaining({ kind: 'message', from: 'person', text: 'Fix the flaky test' }),
    ]);
    expect(starting.outcome).toBeNull();
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
