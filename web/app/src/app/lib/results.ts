import type { Outcome, ReplySources, TurnResult } from '@glave/client';
import { type GlzRunStatus, formatDuration, formatMoney, listFormat } from '@glave/glaze';

import type { ApprovalItem, Draft, MessageItem, QuestionItem, ThreadItem } from './conversation';
import type { ChangedFile } from './files';
import { plural } from './plural';
import type { Turn } from './turns';

/** What waits for the person in a turn: an approval not yet decided, a question not answered. */
export type Waiting = ApprovalItem | QuestionItem;

/** What a turn's work came to, in a line: its status, the facts, and how many steps. */
export interface Receipt {
  readonly status: GlzRunStatus;
  /**
   * The time it took (so far, while it has not ended, unless the working line counts it); and,
   * once it has ended, the server's result: what it cost, or that the cost was not reported,
   * whether that was estimated, where its model calls ran, and how many commands ran outside
   * the sandbox or with the network, when any did.
   */
  readonly facts: readonly string[];
  /** How many steps, in words: "4 steps". */
  readonly steps: string;
}

/** What a turn shows while it works with nothing of its own to show yet. */
export interface Working {
  /**
   * The step it takes now, in the server's words, while a tool runs and the server said which;
   * `null` while its model thinks.
   */
  readonly step: string | null;
  /** When it started, as the server recorded it; `null` until it has. */
  readonly since: string | null;
}

/**
 * A turn as the thread shows it, results first: the person's message; then, for a turn that
 * did work, its reply, the files it changed and its receipt, with the work itself (the calls,
 * what it said along the way, the questions and approvals already answered) folded behind the
 * receipt; and last what waits for the person. A turn that did no work shows what it said in
 * order instead, and no receipt.
 */
export interface TurnResults {
  readonly key: string;
  readonly prompt: MessageItem | null;
  /** In a turn that did no work, what it said before its reply. */
  readonly before: readonly ThreadItem[];
  /** The last thing it said, unless a reply is being written. */
  readonly reply: MessageItem | null;
  /** What that reply drew on from the web, as the server says, when it drew on any. */
  readonly sources: ReplySources | null;
  /** The reply being written, in the turn that runs. */
  readonly live: Draft | null;
  /**
   * While it runs with nothing of its own to show yet (no reply or reasoning streaming, nothing
   * waiting for the person, and not just after the model's last words), what shows instead: the
   * step it takes, when a tool runs, and since when it has worked; else `null`.
   */
  readonly working: Working | null;
  readonly files: readonly ChangedFile[];
  /** The work folded behind the receipt, in order. */
  readonly steps: readonly ThreadItem[];
  /** The receipt, for a turn that did work. */
  readonly receipt: Receipt | null;
  readonly waiting: readonly Waiting[];
}

const STOPPED: readonly Outcome[] = ['cancelled', 'interrupted', 'limit_reached'];

function waits(item: ThreadItem): item is Waiting {
  return (
    (item.kind === 'approval' && item.decision === null) ||
    (item.kind === 'question' && item.answers === null)
  );
}

function isCall(item: ThreadItem): boolean {
  return item.kind === 'call' || item.kind === 'question';
}

function said(item: ThreadItem): item is MessageItem {
  return item.kind === 'message' && item.from === 'model' && item.text.trim() !== '';
}

/** Whether a call of `turn`'s runs now. */
function calling(turn: Turn): boolean {
  return turn.items.some((item) => item.kind === 'call' && item.status === 'running');
}

/**
 * What `turn`, running, shows while it has nothing of its own to show yet: the `step` the server
 * said it takes, while a call runs, and since when it has worked; `null` when it has something.
 */
function workingOn(
  turn: Turn,
  live: Draft | null,
  step: string | null,
  running: boolean,
  waiting: number,
): Working | null {
  const last = turn.items.at(-1);
  const answered = last?.kind === 'message' && last.from === 'model';
  const idle = !running || turn.result !== null || live !== null || waiting > 0 || answered;
  return idle ? null : { step: calling(turn) ? step : null, since: turn.started };
}

function statusOf(turn: Turn, waiting: readonly Waiting[], running: boolean): GlzRunStatus {
  const outcome = turn.result?.outcome ?? null;
  if (outcome === 'completed') {
    return 'done';
  }
  if (outcome === 'failed') {
    return 'failed';
  }
  if (outcome !== null && STOPPED.includes(outcome)) {
    return 'stopped';
  }
  if (waiting.length > 0) {
    return 'waiting';
  }
  return running || calling(turn) ? 'running' : 'done';
}

/** What the turn's model calls cost, as the server says: estimated, or not reported. */
function spent(result: TurnResult, locale: string): string {
  const cost = result.cost ?? null;
  const said =
    cost === null
      ? $localize`:@@results.costNotReported:cost not reported`
      : formatMoney(cost, locale);
  return result.estimated === true
    ? $localize`:@@results.estimated:${said}:COST: (estimated)`
    : said;
}

/**
 * How many commands ran outside the sandbox, and in it with the network, as the server counted
 * them, in words, when any did.
 */
function outsideSandbox(result: TurnResult, locale: string): readonly string[] {
  const ran = result.outside_sandbox ?? 0;
  const network = result.with_network ?? 0;
  return [
    ...(ran === 0
      ? []
      : [
          plural(
            $localize`:@@results.outsideSandbox:{${ran}:COUNT:, plural, one {# command} other {# commands}} outside the sandbox`,
            locale,
          ),
        ]),
    ...(network === 0
      ? []
      : [
          plural(
            $localize`:@@results.withNetwork:{${network}:COUNT:, plural, one {# command} other {# commands}} with the network`,
            locale,
          ),
        ]),
  ];
}

/** The server's result of a turn, in words for `locale`: its cost, where it ran, what ran outside. */
function resultFacts(result: TurnResult, locale: string): readonly string[] {
  const via = result.via ?? [];
  const routes = listFormat(locale, { style: 'short', type: 'unit' }).format(via);
  return [
    spent(result, locale),
    ...(via.length === 0 ? [] : [$localize`:@@results.via:via ${routes}:ROUTES:`]),
    ...outsideSandbox(result, locale),
  ];
}

/**
 * `turn` results first, its facts written for `locale`. `live` is the reply being written, for
 * the turn that runs now (`running`); it takes the reply's place, and what was the reply goes
 * back among what was said along the way. `step` is the step that turn takes now, in the
 * server's words, as it said while a tool runs. `sourced` holds what replies drew on, by the
 * `seq` of each reply.
 */
export function turnResults(
  turn: Turn,
  live: Draft | null,
  step: string | null,
  running: boolean,
  locale: string,
  sourced: ReadonlyMap<number, ReplySources> = new Map(),
): TurnResults {
  const [first, ...after] = turn.items;
  const asked = first?.kind === 'message' && first.from === 'person';
  const prompt = asked ? first : null;
  const rest = asked ? after : turn.items;
  const waiting = rest.filter(waits);
  const others = rest.filter((item) => !waits(item));
  const reply = live === null ? (others.filter(said).at(-1) ?? null) : null;
  const along = others.filter((item) => item !== reply);
  const calls = rest.filter(isCall).length;
  const worked = calls > 0;
  const working = workingOn(turn, live, step, running, waiting.length);
  // While the working line counts the turn's time, the receipt leaves it to it.
  const timed = turn.took !== null && turn.took !== 0 && working === null;
  const facts = [
    ...(timed ? [formatDuration(turn.took, locale)] : []),
    ...(turn.result === null ? [] : resultFacts(turn.result, locale)),
  ];
  return {
    key: turn.key,
    prompt,
    before: worked ? [] : along,
    reply,
    sources: reply === null ? null : (sourced.get(reply.seq) ?? null),
    live,
    working,
    files: turn.files,
    steps: worked ? along : [],
    receipt: worked
      ? {
          status: statusOf(turn, waiting, running),
          facts,
          steps: plural(
            $localize`:@@results.steps:{${calls}:COUNT:, plural, one {# step} other {# steps}}`,
            locale,
          ),
        }
      : null,
    waiting,
  };
}
