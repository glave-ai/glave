import type { ReplySources, ThreadSteps } from '@glave/client';

import type { MessageItem, ThreadItem } from './conversation';
import type { Turn } from './turns';

/** A run of tool calls as the server tells it: its key and its line. */
export interface ToldRun {
  readonly key: string;
  readonly sentence: string;
}

/** What the server says of a thread's steps: each call's run, by the call's id, and the replies. */
export interface Steps {
  readonly runs: ReadonlyMap<string, ToldRun>;
  /** The `seq` of each turn's reply, its last words. */
  readonly replies: ReadonlySet<number>;
}

export const NO_STEPS: Steps = { runs: new Map(), replies: new Set() };

/** `steps` as the server gave them, each call found by its id. */
export function stepsOf(steps: ThreadSteps): Steps {
  const runs = new Map<string, ToldRun>();
  for (const run of steps.runs) {
    const told = { key: `run-${String(run.first)}`, sentence: run.sentence };
    for (const call of run.calls) {
      runs.set(call, told);
    }
  }
  return { runs, replies: new Set(steps.replies) };
}

/**
 * What the server's steps are read again on, as `turns` stand: each call and how it stands, and
 * each turn's end; `null` while no turn has made a call.
 */
export function stepping(turns: readonly Turn[]): string | null {
  const calls = turns.flatMap((turn) => turn.items.filter((item) => item.kind === 'call'));
  if (calls.length === 0) {
    return null;
  }
  const ends = turns.filter((turn) => turn.result !== null).length;
  return [...calls.map((call) => `${call.key}:${call.status}`), String(ends)].join(' ');
}

/** A run of calls in a turn: the server's line, and what it opens to, in order. */
export interface RunPart {
  readonly kind: 'run';
  readonly key: string;
  readonly sentence: string;
  /** Its calls, each approval already answered after its call, and the reasoning between them. */
  readonly items: readonly ThreadItem[];
}

/** Anything else a turn shows in its place: what the model said, a question answered, a call. */
export interface ItemPart {
  readonly kind: 'item';
  readonly key: string;
  readonly item: ThreadItem;
  /** Whether it is a message of the model's said along the way, not its turn's reply. */
  readonly interim: boolean;
  /** What it drew on from the web, when it is a reply that drew on any. */
  readonly sources: ReplySources | null;
}

export type TurnPart = RunPart | ItemPart;

/** The call `item` is, or answers for. */
function callOf(item: ThreadItem): string | null {
  if (item.kind === 'call') {
    return item.call;
  }
  return item.kind === 'approval' ? item.approval : null;
}

/** Whether `item` is the model thinking with nothing said: its reasoning alone. */
function thinking(item: ThreadItem): item is MessageItem {
  return item.kind === 'message' && item.from === 'model' && item.text.trim() === '';
}

/**
 * `items` in the parts a turn shows, in order: each run of calls the server told in a line, its
 * calls and the approvals answered for them inside it, with the model's reasoning between them;
 * everything else in its own place, a call the server has not told yet among them; each message
 * of the model's before the reply the server names marked as said along the way, and a reply
 * with what it drew on, by its `seq` in `sourced`.
 */
export function turnParts(
  items: readonly ThreadItem[],
  steps: Steps,
  sourced: ReadonlyMap<number, ReplySources>,
): readonly TurnPart[] {
  const parts: TurnPart[] = [];
  // Which message is the reply is known once the server says so, as the turn ends.
  const answered = items.some((item) => steps.replies.has(item.seq));
  // Reasoning alone after a run: inside it when the run goes on, else in its own place.
  let thought: MessageItem[] = [];
  const own = (item: ThreadItem): void => {
    const interim =
      answered && item.kind === 'message' && item.from === 'model' && !steps.replies.has(item.seq);
    const sources = sourced.get(item.seq) ?? null;
    parts.push({ kind: 'item', key: item.key, item, interim, sources });
  };
  for (const item of items) {
    if (thinking(item) && parts.at(-1)?.kind === 'run') {
      thought.push(item);
      continue;
    }
    const call = callOf(item);
    const run = call === null ? undefined : steps.runs.get(call);
    const last = parts.at(-1);
    if (run !== undefined && last?.kind === 'run' && last.key === run.key) {
      parts[parts.length - 1] = { ...last, items: [...last.items, ...thought, item] };
      thought = [];
      continue;
    }
    thought.forEach(own);
    thought = [];
    if (run === undefined) {
      own(item);
    } else {
      parts.push({ kind: 'run', key: run.key, sentence: run.sentence, items: [item] });
    }
  }
  thought.forEach(own);
  return parts;
}
