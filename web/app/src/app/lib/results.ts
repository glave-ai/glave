import type { ChangedFile, Outcome, ReplySources, TurnResult } from '@glave/client';
import { type GlzRunStatus, formatDuration, formatMoney, listFormat } from '@glave/glaze';

import type { ApprovalItem, Draft, MessageItem, QuestionItem, ThreadItem } from './conversation';
import { plural } from './plural';
import { NO_STEPS, type Steps, type TurnPart, turnParts } from './runs';
import type { Turn } from './turns';

/** What waits for the person in a turn: an approval not yet decided, a question not answered. */
export type Waiting = ApprovalItem | QuestionItem;

/** What a turn's work came to, in a line: its status, the facts, and how many steps. */
export interface Receipt {
  readonly status: GlzRunStatus;
  /**
   * The time it took (so far, while it has not ended, unless the working line counts it); and,
   * once it has ended, the server's result: what it cost, or that the cost was not reported,
   * whether that was estimated, the model by the server's name for it, and how many commands
   * ran outside the sandbox or with the network, when any did.
   */
  readonly facts: readonly string[];
  /** What the line opens to: where its model calls ran, as the server named them. */
  readonly details: readonly string[];
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
 * A turn as the thread shows it: the person's message; then, in order, what the model said at
 * each step, each run of calls between as the server's line, and the questions and approvals
 * already answered, its reply last; the files it changed; for a turn that did work, its receipt;
 * and last what waits for the person.
 */
export interface TurnResults {
  readonly key: string;
  readonly prompt: MessageItem | null;
  /** What it did, in order, but for what waits for the person. */
  readonly parts: readonly TurnPart[];

  /** The reply being written, in the turn that runs. */
  readonly live: Draft | null;
  /**
   * While it runs with nothing of its own to show yet (no reply or reasoning streaming, nothing
   * waiting for the person, and not just after the model's last words), what shows instead: the
   * step it takes, when a tool runs, and since when it has worked; else `null`.
   */
  readonly working: Working | null;
  /** The files it changed, as the server says; `null` until it has said, as the thread opens. */
  readonly files: readonly ChangedFile[] | null;
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

/**
 * What model calls cost, as the server says of a turn or a thread: estimated, or not reported,
 * written for `locale`.
 */
export function spent(result: Pick<TurnResult, 'cost' | 'estimated'>, locale: string): string {
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

/** Where the server says a turn's model calls ran, in words for `locale`, when it said. */
function routesOf(result: TurnResult | null, locale: string): readonly string[] {
  const via = result?.via ?? [];
  const routes = listFormat(locale, { style: 'short', type: 'unit' }).format(via);
  return via.length === 0 ? [] : [$localize`:@@results.via:via ${routes}:ROUTES:`];
}

/** What the server says of a thread's turns besides their events. */
export interface TurnFacts {
  /** What replies drew on, by the `seq` of each reply. */
  readonly sourced: ReadonlyMap<number, ReplySources>;
  /**
   * The files each turn changed, by the `seq` of its start; `null` until the server has said,
   * as the thread opens.
   */
  readonly changed: ReadonlyMap<number, readonly ChangedFile[]> | null;
  /** Each run of calls in a line, and each turn's reply. */
  readonly steps: Steps;
  /** Each model's name as the server gives it, by the id a turn records. */
  readonly models: ReadonlyMap<string, string>;
}

const NO_FACTS: TurnFacts = {
  sourced: new Map(),
  changed: new Map(),
  steps: NO_STEPS,
  models: new Map(),
};

/** The files `turn` changed, in `changed`, by the `seq` of its start; `null` while not known. */
function filesOf(turn: Turn, changed: TurnFacts['changed']): readonly ChangedFile[] | null {
  if (changed === null) {
    return null;
  }
  return turn.startSeq === null ? [] : (changed.get(turn.startSeq) ?? []);
}

/**
 * `turn` as the thread shows it, its facts written for `locale`. `live` is what the model writes
 * now, for the turn that runs now (`running`), after everything recorded. `step` is the step
 * that turn takes now, in the server's words, as it said while a tool runs. `told` is what the
 * server says of the turn besides its events: its runs of calls, its reply, what the reply drew
 * on, the files it changed and the names of models.
 */
export function turnResults(
  turn: Turn,
  live: Draft | null,
  step: string | null,
  running: boolean,
  locale: string,
  told: TurnFacts = NO_FACTS,
): TurnResults {
  const [first, ...after] = turn.items;
  const asked = first?.kind === 'message' && first.from === 'person';
  const prompt = asked ? first : null;
  const rest = asked ? after : turn.items;
  const waiting = rest.filter(waits);
  const parts = turnParts(
    rest.filter((item) => !waits(item)),
    told.steps,
    told.sourced,
  );
  const calls = rest.filter(isCall).length;
  const working = workingOn(turn, live, step, running, waiting.length);
  // While the working line counts the turn's time, the receipt leaves it to it.
  const timed = turn.took !== null && turn.took !== 0 && working === null;
  const model = turn.model === null ? undefined : told.models.get(turn.model);
  const result = turn.result;
  const facts = [
    ...(timed ? [formatDuration(turn.took, locale)] : []),
    ...(result === null ? [] : [spent(result, locale)]),
    ...(model === undefined ? [] : [model]),
    ...(result === null ? [] : outsideSandbox(result, locale)),
  ];
  return {
    key: turn.key,
    prompt,
    parts,
    live,
    working,
    files: filesOf(turn, told.changed),
    receipt:
      calls > 0
        ? {
            status: statusOf(turn, waiting, running),
            facts,
            details: routesOf(result, locale),
            steps: plural(
              $localize`:@@results.steps:{${calls}:COUNT:, plural, one {# step} other {# steps}}`,
              locale,
            ),
          }
        : null,
    waiting,
  };
}
