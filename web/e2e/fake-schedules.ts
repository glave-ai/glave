import type { RawEvent, ScheduleInfo, ThreadSummary } from '@glave/client';
import type { Page, Route } from '@playwright/test';

import { PERSONAL } from './fake-api';
import { type Answers, SCRATCH, answerFrom, posted } from './fake-records';
import { discovered } from './fake-signin';

/** The chat the person asked in to repeat their email summary. */
export const ASKING = '01900000-0000-7000-8000-0000000005c1';
/** The schedule's own chat, which "Repeat it" makes. */
export const MORNING = '01900000-0000-7000-8000-0000000005c2';
/** The schedule "Repeat it" makes. */
const SCHEDULE = '01900000-0000-7000-8000-0000000005c3';
/** The time zone the schedule repeats in, which the tests' browser is in too. */
export const ZONE = 'Europe/Lisbon';
/** When the schedule repeats, in the server's words. */
export const REPEATS = 'Every weekday at 8:00 am';

const NONE = { input: 0, output: 0, cache_read: 0, cache_write: 0, reasoning: 0 };
const CALL = 'call_3';

/** An event of `thread`, as the server records it, of version `v`. */
function recorded(
  thread: string,
  seq: number,
  type: string,
  data: RawEvent['data'],
  v = 1,
): RawEvent {
  return {
    glave: 1,
    id: `01900000-0000-7000-8000-${String(5000 + seq).padStart(12, '0')}`,
    thread,
    seq,
    at: `2026-11-16T09:00:${String(seq).padStart(2, '0')}.000Z`,
    type,
    v,
    data,
  };
}

/** One of the person's chats, as the lists and `thread/get` show it. */
function chat(id: string, title: string, more: Partial<ThreadSummary> = {}): ThreadSummary {
  return {
    id,
    project: PERSONAL.id,
    status: 'idle',
    standing: 'active',
    title,
    permissions: 'auto',
    origin: { kind: 'web' },
    created_at: '2026-11-16T09:00:00.000Z',
    updated_at: '2026-11-16T09:01:00.000Z',
    turns: 1,
    tokens: NONE,
    workspace: SCRATCH,
    ...more,
  };
}

/** Which card the model asks with: every weekday at 8, or a shape with no plain words. */
export type Card = 'weekdays' | 'unplain';

/** How the schedule's call ends once the person says yes: made, or refused by the server. */
export type Outcome = 'made' | 'refused';

/** The card's lines, typed, as the server records them for each kind of card. */
const LINES = {
  weekdays: {
    when: 'Monday to Friday, 8:00 am your time. First run: Tuesday 17 November.',
    chat: 'Each run is added to its own chat, “Morning email summary”.',
    prompt: 'Each run starts with: “Summarize my email”',
  },
  unplain: {
    when: 'Times are in your time zone.',
    chat: 'Each run is added to its own chat, “Morning email summary”.',
    prompt: 'Each run starts with: “Summarize my email”',
  },
} as const;

/** The server's question and what it was written from, for each kind of card. */
const ASKS = {
  weekdays: {
    ask: 'Repeat this every weekday at 8:00 am?',
    asking: { says: 'ask-repeat', when: 'every weekday at 8:00 am' },
    cron: '0 8 * * 1-5',
  },
  unplain: {
    ask: 'Repeat this, next on Monday 30 November, Thursday 31 December and Sunday 31 January?',
    asking: {
      says: 'ask-repeat-next',
      runs: 'Monday 30 November, Thursday 31 December and Sunday 31 January',
    },
    cron: '0 8 30,31 * *',
  },
} as const;

/** What the asking chat records: the person's ask, the model's call, and the server's card. */
function asked(card: Card): readonly RawEvent[] {
  const lines = LINES[card];
  const detail = [lines.when, lines.chat, lines.prompt].join('\n');
  const { ask, asking, cron } = ASKS[card];
  const model = { provider: 'openrouter', name: 'hope' };
  return [
    recorded(ASKING, 1, 'turn.started', { model }, 2),
    recorded(ASKING, 2, 'message.added', {
      role: 'user',
      parts: [{ type: 'text', text: 'Summarize my email every weekday at 8.' }],
    }),
    recorded(ASKING, 3, 'tool.call.requested', {
      call_id: CALL,
      tool: 'schedule',
      arguments: {
        json: {
          action: 'create',
          name: 'Morning email summary',
          prompt: 'Summarize my email',
          cron,
        },
      },
      summary: 'schedule: Morning email summary',
    }),
    recorded(
      ASKING,
      4,
      'approval.requested',
      {
        call_id: CALL,
        tool: 'schedule',
        summary: `schedule: ${detail}`,
        ask,
        asking,
        detail,
        secret: false,
        caution: 'It runs only while glave runs on this computer.',
        reason: { risk: { says: 'risk-runs-while-glave-runs' } },
        repeating: {
          cron,
          zone: ZONE,
          name: 'Morning email summary',
          first: '2026-11-17T08:00:00.000Z',
          prompt: 'Summarize my email',
        },
        lines,
      },
      2,
    ),
  ];
}

/** What the asking chat records once the person answers `decision`, the call ending `outcome`. */
function answered(decision: string, outcome: Outcome): readonly RawEvent[] {
  const yes = decision === 'allow_once';
  const made = yes && outcome === 'made';
  const result = made
    ? {
        is_error: false,
        output: [{ type: 'text', text: `${REPEATS}, in its own chat.` }],
        structured: { schedule: SCHEDULE, chat: MORNING, repeats: REPEATS },
      }
    : {
        is_error: true,
        output: [{ type: 'text', text: yes ? 'As many as may be kept.' : 'Not now.' }],
      };
  return [
    recorded(ASKING, 5, 'approval.decided', { call_id: CALL, decision }),
    recorded(ASKING, 6, 'tool.call.completed', { call_id: CALL, ...result }),
    recorded(ASKING, 7, 'message.added', {
      role: 'assistant',
      parts: [{ type: 'text', text: made ? 'Done: it comes every weekday at 8.' : 'All right.' }],
    }),
    recorded(ASKING, 8, 'turn.ended', { outcome: 'completed' }),
  ];
}

/** What the fake keeps while it runs. */
interface Kept {
  readonly card: Card;
  /** How the schedule's call ends once the person says yes. */
  readonly outcome: Outcome;
  /** The person's answer to the card, once given. */
  decision: string | null;
  /** The lists' watch the page holds open, answered once the schedule's chat is made. */
  watch: { readonly route: Route; readonly id: number } | null;
  /** Each time zone the page gave, in order. */
  readonly zones: string[];
}

/** Whether the schedule and its chat were made. */
function made(kept: Kept): boolean {
  return kept.decision === 'allow_once' && kept.outcome === 'made';
}

/** The schedule, as the server shows it, running next tomorrow at 8. */
function info(): ScheduleInfo {
  return {
    schedule: SCHEDULE,
    name: 'Morning email summary',
    name_shown: 'Morning email summary',
    stop: 'Stop repeating Morning email summary?',
    project: PERSONAL.id,
    thread: MORNING,
    from: ASKING,
    prompt: 'Summarize my email',
    prompt_shown: 'Summarize my email',
    when: { cron: ASKS.weekdays.cron },
    zone: ZONE,
    repeats: REPEATS,
    budget: 200_000,
    created_at: '2026-11-16T09:01:00.000Z',
    paused: false,
    next: '2026-11-17T08:00:00.000Z',
  };
}

/** The schedule's chat once made, carrying its schedule. */
function morningChat(kept: Kept): ThreadSummary | undefined {
  return made(kept) ? chat(MORNING, 'Morning email summary', { schedule: info() }) : undefined;
}

const ASKING_CHAT = chat(ASKING, 'Summarize my email');

function events(thread: string, kept: Kept): readonly RawEvent[] | undefined {
  if (thread !== ASKING) {
    return undefined;
  }
  const before = asked(kept.card);
  return kept.decision === null ? before : [...before, ...answered(kept.decision, kept.outcome)];
}

/** A thread's summary, as `thread/get` answers it, for the threads this fake has. */
function summaryOf(thread: string, kept: Kept): ThreadSummary | undefined {
  return thread === ASKING ? ASKING_CHAT : thread === MORNING ? morningChat(kept) : undefined;
}

/** Tells the lists' watch the page holds of the schedule's chat, once made, and ends it. */
async function tell(kept: Kept): Promise<void> {
  const [watch, morning] = [kept.watch, morningChat(kept)];
  if (watch === null || morning === undefined) {
    return;
  }
  kept.watch = null;
  const messages = [
    { jsonrpc: '2.0', method: 'thread/changed', params: { thread: MORNING, summary: morning } },
    { jsonrpc: '2.0', id: watch.id, result: { ended: 'unwatched' } },
  ];
  await watch.route.fulfill({
    contentType: 'text/event-stream',
    body: messages.map((message) => `data: ${JSON.stringify(message)}\n\n`).join(''),
  });
}

const ANSWERS: Answers<Kept> = {
  'server/discover': () => ({ ...discovered(), schedules: true }),
  'zone/set': ({ zone }, kept) => {
    kept.zones.push(zone);
    return {};
  },
  'thread/list': ({ project }, kept) => {
    const morning = morningChat(kept);
    const threads = morning === undefined ? [ASKING_CHAT] : [morning, ASKING_CHAT];
    return project === PERSONAL.id ? { threads } : undefined;
  },
  'thread/get': ({ thread }, kept) => {
    const summary = summaryOf(thread, kept);
    return summary === undefined ? undefined : { thread: summary };
  },
  'thread/turns': ({ thread }, kept) => {
    const all = events(thread, kept);
    return all === undefined ? undefined : { events: all };
  },
  'thread/events': ({ thread }, kept) => {
    const all = events(thread, kept);
    return all === undefined ? undefined : { events: all };
  },
  'thread/steps': ({ thread }, kept) =>
    thread === ASKING
      ? {
          steps: {
            runs: [{ first: 3, calls: [CALL], sentence: 'Set up a schedule' }],
            replies: kept.decision === null ? [] : [7],
          },
        }
      : undefined,
  'approval/decide': ({ thread, decision }, kept) => {
    if (thread !== ASKING) {
      return undefined;
    }
    kept.decision = decision;
    return {};
  },
};

/**
 * A server where schedules run, over whatever `fakeApi` answers: the person's chat in which the
 * model asks to repeat their email summary, the server's card waiting for them (`card`: every
 * weekday at 8, or a shape with no plain words), recorded as the server records it; "Repeat it"
 * makes the schedule and its own chat, which the lists' watch is told of, or with `outcome`
 * `refused` the server refuses it; "Not now" makes nothing. Each time zone the page gives is
 * kept in `zones`.
 */
export async function schedules(
  page: Page,
  {
    card = 'weekdays',
    outcome = 'made',
  }: { readonly card?: Card; readonly outcome?: Outcome } = {},
): Promise<Kept> {
  const kept: Kept = { card, outcome, decision: null, watch: null, zones: [] };
  await page.route('**/*', async (route: Route) => {
    const message = await posted(route);
    if (message === null) {
      return;
    }
    if (message.method === 'thread/list/watch') {
      kept.watch = { route, id: message.id };
      await tell(kept);
      return;
    }
    const result = answerFrom(ANSWERS, message.method, message.params, kept);
    if (result === undefined) {
      await route.fallback();
      return;
    }
    await route.fulfill({
      contentType: 'application/json',
      body: JSON.stringify({ jsonrpc: '2.0', id: message.id, result }),
    });
    if (message.method === 'approval/decide') {
      await tell(kept);
    }
  });
  return kept;
}
