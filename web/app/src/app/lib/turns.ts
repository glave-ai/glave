import type { RawEvent } from '@glave/client';
import type { GlzMoney } from '@glave/glaze';

import { type ThreadItem, threadItems } from './conversation';
import { type ChangedFile, changedFiles } from './files';

/** How a turn ended, as its `turn.ended` says. */
export type TurnOutcome = 'completed' | 'cancelled' | 'failed' | 'interrupted' | 'limit_reached';

const OUTCOMES: readonly string[] = [
  'completed',
  'cancelled',
  'failed',
  'interrupted',
  'limit_reached',
];

/** One turn of a thread: what the person asked, and everything the model did for it. */
export interface Turn {
  readonly key: string;
  /** Its items in order, the person's message first when it has one. */
  readonly items: readonly ThreadItem[];
  /** The files its own `write` and `edit` calls changed. */
  readonly files: readonly ChangedFile[];
  /**
   * How long it took, in milliseconds: to its end, or while it has not ended (it runs, or waits
   * for the person) to the latest thing it recorded. `null` with no start recorded.
   */
  readonly took: number | null;
  readonly outcome: TurnOutcome | null;
  /** What its model calls cost, when every one of them said, in one currency. */
  readonly cost: GlzMoney | null;
  /**
   * Where its model calls ran, as the server named them, in order and once each: the service,
   * and the provider it passed them on to (`OpenRouter (Anthropic)`). Empty when none said.
   */
  readonly routes: readonly string[];
}

type Data = Readonly<Record<string, unknown>>;

function isRecord(value: unknown): value is Data {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

function isOutcome(value: unknown): value is TurnOutcome {
  return typeof value === 'string' && OUTCOMES.includes(value);
}

function byPerson(event: RawEvent): boolean {
  return event.type === 'message.added' && event.data['role'] === 'user';
}

/** Events that put something in the conversation besides the person's message. */
const WORK: ReadonlySet<string> = new Set([
  'message.added',
  'tool.call.requested',
  'approval.requested',
]);

/**
 * Whether `event` starts a new turn after `turn`'s events: a person's message after anything
 * but a bare `turn.started`, or a `turn.started` after anything but the person's message alone
 * (glave records the start just before the message; older threads, just after).
 */
function starts(event: RawEvent, turn: readonly RawEvent[]): boolean {
  const started = turn.some((each) => each.type === 'turn.started');
  const asked = turn.some(byPerson);
  const worked = turn.some((each) => WORK.has(each.type) && !byPerson(each));
  if (byPerson(event)) {
    return asked || worked;
  }
  return event.type === 'turn.started' && (started || worked);
}

/** What the model calls in `events` cost, when each said, in one currency; else `null`. */
function costOf(events: readonly RawEvent[]): GlzMoney | null {
  const responses = events.filter((event) => event.type === 'model.responded');
  const amounts = responses.map((event) => {
    const cost = event.data['cost'];
    const amount = isRecord(cost) ? cost['amount'] : undefined;
    return isRecord(amount) &&
      typeof amount['micros'] === 'number' &&
      typeof amount['currency'] === 'string'
      ? { micros: amount['micros'], currency: amount['currency'] }
      : null;
  });
  const [first] = amounts;
  if (first === undefined || first === null) {
    return null;
  }
  let micros = 0;
  for (const amount of amounts) {
    if (amount?.currency !== first.currency) {
      return null;
    }
    micros += amount.micros;
  }
  return { micros, currency: first.currency };
}

/** Where the model calls in `events` ran, once each, in order, as their records name them. */
function routesOf(events: readonly RawEvent[]): readonly string[] {
  const routes: string[] = [];
  for (const event of events) {
    if (event.type !== 'model.responded') {
      continue;
    }
    const service = event.data['service'];
    const upstream = event.data['upstream'];
    if (typeof service !== 'string' || service === '') {
      continue;
    }
    const route =
      typeof upstream === 'string' && upstream !== '' && upstream !== service
        ? `${service} (${upstream})`
        : service;
    if (!routes.includes(route)) {
      routes.push(route);
    }
  }
  return routes;
}

function turnOf(events: readonly RawEvent[], items: readonly ThreadItem[]): Turn {
  const started = events.find((event) => event.type === 'turn.started');
  const ended = events.filter((event) => event.type === 'turn.ended').at(-1);
  const outcome = ended?.data['outcome'];
  const until = ended ?? events.at(-1);
  const took =
    started === undefined || until === undefined
      ? null
      : Math.max(0, Date.parse(until.at) - Date.parse(started.at));
  return {
    key: `turn-${String(events[0]?.seq ?? 0)}`,
    items,
    files: changedFiles(events),
    took: Number.isNaN(took) ? null : took,
    outcome: isOutcome(outcome) ? outcome : null,
    cost: costOf(events),
    routes: routesOf(events),
  };
}

/**
 * A thread's turns, from its events, in order: each from the person's message (or the
 * `turn.started` just before it) to the next, with its items, the files it changed, how long
 * it took and how it ended, and what it cost. Events before the first message that put nothing
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
 * it shows at once, with glave working under it.
 */
export function startingTurn(prompt: string): Turn {
  return {
    key: 'turn-starting',
    items: [
      {
        kind: 'message',
        key: 'starting',
        seq: 0,
        from: 'person',
        text: prompt,
        reasoning: '',
        steered: false,
        files: [],
      },
    ],
    files: [],
    took: null,
    outcome: null,
    cost: null,
    routes: [],
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
