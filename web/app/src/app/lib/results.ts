import { type GlzRunStatus, formatDuration, formatMoney } from '@glave/glaze';

import type { ApprovalItem, Draft, MessageItem, QuestionItem, ThreadItem } from './conversation';
import type { ChangedFile } from './files';
import { plural } from './plural';
import type { Turn, TurnOutcome } from './turns';

/** What waits for the person in a turn: an approval not yet decided, a question not answered. */
export type Waiting = ApprovalItem | QuestionItem;

/** What a turn's work came to, in a line: its status, the facts, and how many steps. */
export interface Receipt {
  readonly status: GlzRunStatus;
  /**
   * The time it took (so far, while it has not ended) and what it cost, as far as known, and
   * how many commands ran outside the sandbox, when any did.
   */
  readonly facts: readonly string[];
  /** How many steps, in words: "4 steps". */
  readonly steps: string;
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
  /** The reply being written, in the turn that runs. */
  readonly live: Draft | null;
  /**
   * Whether the turn runs and waits on its model with nothing to show yet: no reply or
   * reasoning streaming, no call running, nothing waiting for the person, and not just after
   * the model's last words.
   */
  readonly working: boolean;
  readonly files: readonly ChangedFile[];
  /** The work folded behind the receipt, in order. */
  readonly steps: readonly ThreadItem[];
  /** The receipt, for a turn that did work. */
  readonly receipt: Receipt | null;
  readonly waiting: readonly Waiting[];
}

const STOPPED: readonly TurnOutcome[] = ['cancelled', 'interrupted', 'limit_reached'];

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

/** Whether `turn`, running, waits on its model with nothing of it to show yet. */
function workingOn(turn: Turn, live: Draft | null, running: boolean, waiting: number): boolean {
  const last = turn.items.at(-1);
  const answered = last?.kind === 'message' && last.from === 'model';
  const going = turn.items.some((item) => item.kind === 'call' && item.status === 'running');
  return running && turn.outcome === null && live === null && waiting === 0 && !going && !answered;
}

function statusOf(turn: Turn, waiting: readonly Waiting[], running: boolean): GlzRunStatus {
  if (turn.outcome === 'completed') {
    return 'done';
  }
  if (turn.outcome === 'failed') {
    return 'failed';
  }
  if (turn.outcome !== null && STOPPED.includes(turn.outcome)) {
    return 'stopped';
  }
  if (waiting.length > 0) {
    return 'waiting';
  }
  const going = turn.items.some((item) => item.kind === 'call' && item.status === 'running');
  return running || going ? 'running' : 'done';
}

/** How many commands the person let run outside the sandbox, in words, when any were. */
function outsideSandbox(items: readonly ThreadItem[], locale: string): readonly string[] {
  const ran = items.filter(
    (item) =>
      item.kind === 'approval' &&
      item.outsideSandbox &&
      (item.decision === 'allow_once' || item.decision === 'allow_for_thread'),
  ).length;
  return ran === 0
    ? []
    : [
        plural(
          $localize`:@@results.outsideSandbox:{${ran}:COUNT:, plural, one {# command} other {# commands}} outside the sandbox`,
          locale,
        ),
      ];
}

/**
 * `turn` results first, its facts written for `locale`. `live` is the reply being written, for
 * the turn that runs now (`running`); it takes the reply's place, and what was the reply goes
 * back among what was said along the way.
 */
export function turnResults(
  turn: Turn,
  live: Draft | null,
  running: boolean,
  locale: string,
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
  const routes = new Intl.ListFormat(locale, { style: 'short', type: 'unit' }).format(turn.routes);
  const facts = [
    ...(turn.took === null || turn.took === 0 ? [] : [formatDuration(turn.took, locale)]),
    ...(turn.cost === null ? [] : [formatMoney(turn.cost, locale)]),
    ...(turn.routes.length === 0 ? [] : [$localize`:@@results.via:via ${routes}:ROUTES:`]),
    ...outsideSandbox(rest, locale),
  ];
  return {
    key: turn.key,
    prompt,
    before: worked ? [] : along,
    reply,
    live,
    working: workingOn(turn, live, running, waiting.length),
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
