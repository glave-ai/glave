import type { Money, Outcome, RawEvent, ThreadSummary, TurnResult } from '@glave/client';

import { NO_THOUGHT, type ThreadItem, byGlave, threadItems } from './conversation';

const OUTCOMES: readonly string[] = [
  'completed',
  'cancelled',
  'failed',
  'interrupted',
  'limit_reached',
] satisfies readonly Outcome[];

/** One turn of a thread: what the person asked, and everything the model did for it. */
export interface Turn {
  readonly key: string;
  /** Its items in order, the person's message first when it has one. */
  readonly items: readonly ThreadItem[];
  /**
   * The `seq` of its `turn.started`, by which the server names it; `null` with no start
   * recorded.
   */
  readonly startSeq: number | null;
  /**
   * How long it took, in milliseconds: to its end, or while it has not ended (it runs, or waits
   * for the person) to the latest thing it recorded. `null` with no start recorded.
   */
  readonly took: number | null;
  /** When it started, as the server recorded it (`turn.started`); `null` with no start recorded. */
  readonly started: string | null;
  /** The model it started with, by the id its `turn.started` records; `null` with none. */
  readonly model: string | null;
  /**
   * How it ended and what it did and cost, as the server says in its `turn.ended`; `null` until
   * it has ended.
   */
  readonly result: TurnResult | null;
}

type Data = Readonly<Record<string, unknown>>;

function isRecord(value: unknown): value is Data {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

function isOutcome(value: unknown): value is Outcome {
  return typeof value === 'string' && OUTCOMES.includes(value);
}

function isMoney(value: unknown): value is Money {
  return (
    isRecord(value) && typeof value['micros'] === 'number' && typeof value['currency'] === 'string'
  );
}

/**
 * The server's result of a turn, as its `turn.ended` records it: what a client that ran the turn
 * is answered with. A count it does not hold is none; `null` when it says no outcome.
 */
function resultOf(data: Data): TurnResult | null {
  const outcome = data['outcome'];
  if (!isOutcome(outcome)) {
    return null;
  }
  const count = (name: string): number => {
    const value = data[name];
    return typeof value === 'number' ? value : 0;
  };
  const cost = data['cost'];
  return {
    outcome,
    model_calls: count('model_calls'),
    tool_calls: count('tool_calls'),
    outside_sandbox: count('outside_sandbox'),
    with_network: count('with_network'),
    cost: isMoney(cost) ? cost : null,
    estimated: data['estimated'] === true,
    via: texts(data['via']),
    changed: texts(data['changed']),
  };
}

/** The strings of `value`, a list of them; none when it is not one. */
function texts(value: unknown): readonly string[] {
  return Array.isArray(value) ? value.filter((each) => typeof each === 'string') : [];
}

/** Whether `event` records a message of the person's own, not a note glave wrote on their side. */
function byPerson(event: RawEvent): boolean {
  return event.type === 'message.added' && event.data['role'] === 'user' && !byGlave(event.data);
}

/** Whether `event` is the person asking for work: their message, or their Build it of a plan. */
export function asks(event: RawEvent): boolean {
  return byPerson(event) || event.type === 'plan.approved';
}

/** Events that put something in the conversation besides the person's message. */
const WORK: ReadonlySet<string> = new Set([
  'message.added',
  'tool.call.requested',
  'approval.requested',
]);

/**
 * Whether `event` starts a new turn after `turn`'s events: the person asking (`asks`) after
 * anything but a bare `turn.started`, or a `turn.started` after anything but the person's
 * asking alone (glave records the start just before the message, and just after a Build it;
 * older threads, just after the message).
 */
function starts(event: RawEvent, turn: readonly RawEvent[]): boolean {
  const started = turn.some((each) => each.type === 'turn.started');
  const asked = turn.some(asks);
  const worked = turn.some((each) => WORK.has(each.type) && !byPerson(each));
  if (asks(event)) {
    return asked || worked;
  }
  return event.type === 'turn.started' && (started || worked);
}

/** The id of the model a `turn.started` records, `{ provider, name }`: its name. */
function modelOf(model: unknown): string | null {
  const name = isRecord(model) ? model['name'] : undefined;
  return typeof name === 'string' && name !== '' ? name : null;
}

function turnOf(events: readonly RawEvent[], items: readonly ThreadItem[]): Turn {
  const started = events.find((event) => event.type === 'turn.started');
  const ended = events.filter((event) => event.type === 'turn.ended').at(-1);
  const until = ended ?? events.at(-1);
  const took =
    started === undefined || until === undefined
      ? null
      : Math.max(0, Date.parse(until.at) - Date.parse(started.at));
  return {
    key: `turn-${String(events[0]?.seq ?? 0)}`,
    items,
    startSeq: started?.seq ?? null,
    took: Number.isNaN(took) ? null : took,
    started: started?.at ?? null,
    model: modelOf(started?.data['model']),
    result: ended === undefined ? null : resultOf(ended.data),
  };
}

/**
 * A thread's turns, from its events, in order: each from the person's message (or the
 * `turn.started` just before it) to the next, with its items, where it started, how long it
 * took, and its result as the server recorded it. Events before the first message that put nothing
 * in the conversation (the thread's start, its workspace) make no turn of their own.
 */
export function threadTurns(events: readonly RawEvent[]): readonly Turn[] {
  const groups: RawEvent[][] = [];
  for (const event of events) {
    const last = groups.at(-1);
    if (last === undefined || starts(event, last)) {
      groups.push([event]);
    } else {
      last.push(event);
    }
  }
  const items = threadItems(events);
  return groups
    .map((group, index) => {
      const from = group[0]?.seq ?? 0;
      const to = groups[index + 1]?.[0]?.seq ?? Infinity;
      return turnOf(
        group,
        items.filter((item) => item.seq >= from && item.seq < to),
      );
    })
    .filter((turn) => turn.items.length > 0);
}

/**
 * The turn the person just asked for, before the server records it: their message alone, so
 * it shows at once, with glave working under it, keyed `key`.
 */
function startingTurn(prompt: string, key: string): Turn {
  return {
    key,
    items: [
      {
        kind: 'message',
        key: 'starting',
        seq: 0,
        from: 'person',
        text: prompt,
        reasoning: '',
        thought: NO_THOUGHT,
        steered: false,
        files: [],
      },
    ],
    startSeq: null,
    took: null,
    started: null,
    model: null,
    result: null,
  };
}

/** Whether a turn runs in the thread, as its events say: its latest turn started and has not ended. */
export function turnRuns(events: readonly RawEvent[]): boolean {
  for (let at = events.length - 1; at >= 0; at -= 1) {
    const type = events[at]?.type;
    if (type === 'turn.ended') {
      return false;
    }
    if (type === 'turn.started') {
      return true;
    }
  }
  return false;
}

/**
 * Whether a turn runs in the thread, as the server's summary of it says, with its events read
 * before: one does, or waits for the person (not a project's ask, which waits with no turn).
 */
export function summaryRuns(summary: ThreadSummary, events: readonly RawEvent[]): boolean {
  return summary.status === 'running' || (summary.status === 'waiting' && turnRuns(events));
}

/** The `seq` of the latest turn's start, if a turn ever started. */
export function latestStart(events: readonly RawEvent[]): number | null {
  for (let at = events.length - 1; at >= 0; at -= 1) {
    const event = events[at];
    if (event?.type === 'turn.started') {
      return event.seq;
    }
  }
  return null;
}

/** The key of a turn a view asked for after the event `seq` `after`, from asking on. */
function askedKey(after: number): string {
  return `turn-asked-${String(after)}`;
}

/**
 * `turns` as the view that asked for some of them shows them: each turn it asked for, the first
 * recorded after each `seq` in `asked` (the last event it had read as it asked), keyed by that
 * asking from the moment it asked, so what shows of the turn carries on as the server records it
 * a piece at a time; and `prompt`, asked last and not recorded yet, as that turn starting.
 */
export function askedTurns(
  turns: readonly Turn[],
  asked: readonly number[],
  prompt: string | null,
): readonly Turn[] {
  const keyed = [...turns];
  let from = 0;
  let waiting: number | null = null;
  for (const after of asked) {
    const at = turns.findIndex((turn, index) => index >= from && (turn.items[0]?.seq ?? 0) > after);
    const found = turns[at];
    if (found === undefined) {
      waiting = after;
      continue;
    }
    keyed[at] = { ...found, key: askedKey(after) };
    from = at + 1;
  }
  if (prompt === null) {
    return keyed;
  }
  return [...keyed, startingTurn(prompt, waiting === null ? 'turn-starting' : askedKey(waiting))];
}
