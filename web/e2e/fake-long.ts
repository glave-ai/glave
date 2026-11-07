import type { RawEvent } from '@glave/client';

import { DEFAULT_MODEL } from './fake-models';
import { type Answers, bound, call, done, event } from './fake-records';

/**
 * A thread of over 10,000 events, as a long run of work on an always-on computer leaves one: its
 * history read as the server reads it, the latest 20 turns back from where the page asks
 * (`thread/turns`), or 500 events at a time after a `seq` (`thread/events`).
 */
export const LONG = '01900000-0000-7000-8000-000000000141';

/** Its turns, each of 8 events: started, asked, a call and its result, two answers, ended. */
export const LONG_TURNS = 1_250;

/** How many turns a page of `thread/turns` holds, as the server's. */
const TURNS = 20;
/** How many events a page of `thread/events` holds, as the server's when no limit is asked. */
const PAGE = 500;

/** The events of turn `turn` (from 1), from `seq` on. */
function turnOf(turn: number, seq: number): readonly RawEvent[] {
  const model = { provider: 'e2e', name: DEFAULT_MODEL };
  const text = (role: string, said: string): RawEvent['data'] => ({
    role,
    parts: [{ type: 'text', text: said }],
  });
  return [
    event(seq, 'turn.started', { model }, LONG),
    event(seq + 1, 'message.added', text('user', `Step ${String(turn)}, please.`), LONG),
    call(seq + 2, `call_${String(turn)}`, 'bash', { command: 'cargo test' }, LONG),
    done(seq + 3, `call_${String(turn)}`, 'test result: ok', LONG),
    event(seq + 4, 'model.responded', { model, stop: 'tool_use' }, LONG),
    event(seq + 5, 'message.added', text('assistant', `Step ${String(turn)} is done.`), LONG),
    event(seq + 6, 'model.responded', { model, stop: 'end_turn' }, LONG),
    event(
      seq + 7,
      'turn.ended',
      {
        outcome: 'completed',
        model_calls: 2,
        tool_calls: 1,
        outside_sandbox: 0,
        with_network: 0,
        tokens: { input: 900, cache_read: 0, cache_write: 0, output: 60, reasoning: 0 },
      },
      LONG,
    ),
  ];
}

const HISTORY: readonly RawEvent[] = [
  event(1, 'thread.started', {}, LONG),
  bound(2, LONG),
  ...Array.from({ length: LONG_TURNS }, (_, index) => turnOf(index + 1, 3 + index * 8)).flat(),
];

/** The calls about the long thread, which the fake answers as the server reads a log. */
export const ABOUT_LONG: Answers<null> = {
  'thread/turns': ({ thread, before }) => {
    if (thread !== LONG) {
      return undefined;
    }
    const until = HISTORY.filter(
      (each) => before === undefined || before === null || each.seq < before,
    );
    const starts = until.filter((each) => each.type === 'turn.started');
    const from = starts.length > TURNS ? starts.at(-TURNS)?.seq : undefined;
    return from === undefined
      ? { events: until }
      : { events: until.filter((each) => each.seq >= from), earlier: from };
  },
  'thread/events': ({ thread, after }) => {
    if (thread !== LONG) {
      return undefined;
    }
    const events = HISTORY.filter((each) => each.seq > (after ?? 0)).slice(0, PAGE);
    const next = events.length === PAGE ? events.at(-1)?.seq : undefined;
    return next === undefined ? { events } : { events, next };
  },
};
