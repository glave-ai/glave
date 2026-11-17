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
/** What the server refuses the schedule with, after the person's yes, when a test asks. */
export const REFUSED = 'You have as many schedules as you may keep: 10.';

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

const LINES = [
  'Monday to Friday, 8:00 am your time. First run: Tuesday 17 November.',
  'Each run is added to its own chat, “Morning email summary”.',
];

/** What the asking chat records: the person's ask, the model's call, and the server's card. */
const ASKED: readonly RawEvent[] = [
  recorded(ASKING, 1, 'turn.started', { model: { provider: 'openrouter', name: 'hope' } }, 2),
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
        cron: '0 8 * * 1-5',
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
      summary: `schedule: ${LINES.join('\n')}`,
      ask: 'Repeat this every weekday at 8:00 am?',
      asking: { says: 'ask-repeat', when: 'every weekday at 8:00 am' },
      detail: LINES.join('\n'),
      secret: false,
      caution: 'It runs only while glave runs on this computer.',
      reason: { risk: { says: 'risk-runs-while-glave-runs' } },
      repeating: {
        cron: '0 8 * * 1-5',
        zone: ZONE,
        name: 'Morning email summary',
        first: '2026-11-17T08:00:00.000Z',
      },
    },
    2,
  ),
];

/** How the schedule's call ends after the person's answer: made, refused, or declined. */
export type Outcome = 'made' | 'refused';

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
        output: [{ type: 'text', text: yes ? REFUSED : 'The person said not now.' }],
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

/** A run of the schedule in its chat, due at `due`, from `seq`: its prompt, and its reply. */
function run(seq: number, due: string, reply: string): readonly RawEvent[] {
  const model = { provider: 'openrouter', name: 'hope' };
  return [
    recorded(MORNING, seq, 'turn.started', { model, scheduled: due }, 2),
    recorded(MORNING, seq + 1, 'message.added', {
      role: 'user',
      parts: [{ type: 'text', text: 'Summarize my email' }],
    }),
    recorded(MORNING, seq + 2, 'message.added', {
      role: 'assistant',
      parts: [{ type: 'text', text: reply }],
    }),
    recorded(MORNING, seq + 3, 'turn.ended', { outcome: 'completed' }),
  ];
}

/** The schedule's chat: last Friday's run, and this Monday's. */
const RAN: readonly RawEvent[] = [
  ...run(1, '2026-11-13T08:00:00.000Z', 'Nothing urgent.'),
  ...run(5, '2026-11-16T08:00:00.000Z', 'Three need a reply today.'),
];

/** What the fake keeps while it runs. */
interface Kept {
  /** How the schedule's call ends once the person says yes. */
  readonly outcome: Outcome;
  /** Whether the schedule and its chat were made before the page opened. */
  readonly existing: boolean;
  /** The person's answer to the card, once given. */
  decision: string | null;
  paused: boolean;
  removed: boolean;
  /** Each time zone the page gave, in order. */
  readonly zones: string[];
}

/** Whether the schedule and its chat were made. */
function made(kept: Kept): boolean {
  return kept.existing || (kept.decision === 'allow_once' && kept.outcome === 'made');
}

/** The schedule, as the server shows it: running next tomorrow at 8, or paused. */
function info(paused: boolean): ScheduleInfo {
  return {
    schedule: SCHEDULE,
    name: 'Morning email summary',
    project: PERSONAL.id,
    thread: MORNING,
    from: ASKING,
    prompt: 'Summarize my email',
    when: { cron: '0 8 * * 1-5' },
    zone: ZONE,
    repeats: REPEATS,
    budget: 200_000,
    created_at: '2026-11-13T07:00:00.000Z',
    paused,
    ...(paused ? {} : { next: '2026-11-17T08:00:00.000Z' }),
    last: { due: '2026-11-16T08:00:00.000Z', status: 'ended', outcome: 'completed', said: 'Done' },
  };
}

/** The schedule while it is there: none once removed, or before it was made. */
function scheduleOf(kept: Kept): ScheduleInfo | null {
  return made(kept) && !kept.removed ? info(kept.paused) : null;
}

/** The schedule's chat once made, carrying its schedule while it has one. */
function morningChat(kept: Kept): ThreadSummary | undefined {
  if (!made(kept)) {
    return undefined;
  }
  const schedule = scheduleOf(kept);
  return chat(MORNING, 'Morning email summary', schedule === null ? {} : { schedule });
}

function events(thread: string, kept: Kept): readonly RawEvent[] | undefined {
  if (thread === MORNING) {
    return made(kept) ? RAN : undefined;
  }
  if (thread !== ASKING) {
    return undefined;
  }
  return kept.decision === null ? ASKED : [...ASKED, ...answered(kept.decision, kept.outcome)];
}

const ASKING_CHAT = chat(ASKING, 'Summarize my email');

/** A thread's summary, as `thread/get` answers it, for the threads this fake has. */
function summaryOf(thread: string, kept: Kept): ThreadSummary | undefined {
  return thread === ASKING ? ASKING_CHAT : thread === MORNING ? morningChat(kept) : undefined;
}

const CARD: Answers<Kept> = {
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
  'thread/steps': ({ thread }, kept) => {
    if (thread === MORNING) {
      return { steps: { runs: [], replies: [3, 7] } };
    }
    return thread === ASKING
      ? {
          steps: {
            runs: [{ first: 3, calls: [CALL], sentence: 'Set up a schedule' }],
            replies: kept.decision === null ? [] : [7],
          },
        }
      : undefined;
  },
  'schedule/list': (_, kept) => {
    const schedule = scheduleOf(kept);
    return { schedules: schedule === null ? [] : [schedule] };
  },
  'schedule/pause': ({ paused }, kept) => {
    kept.paused = paused;
    return { schedule: info(paused) };
  },
  'schedule/delete': (_, kept) => {
    kept.removed = true;
    return {};
  },
  'approval/decide': ({ thread, decision }, kept) => {
    if (thread !== ASKING) {
      return undefined;
    }
    kept.decision = decision;
    return {};
  },
};

/**
 * Answers `page`'s calls with `answers` over whatever `fakeApi` answers, keeping `kept`: what
 * the answers leave unanswered falls back to it.
 */
async function answering<K>(page: Page, answers: Answers<K>, kept: K): Promise<void> {
  await page.route('**/*', async (route: Route) => {
    const message = await posted(route);
    if (message === null) {
      return;
    }
    const result = answerFrom(answers, message.method, message.params, kept);
    if (result === undefined) {
      await route.fallback();
      return;
    }
    await route.fulfill({
      contentType: 'application/json',
      body: JSON.stringify({ jsonrpc: '2.0', id: message.id, result }),
    });
  });
}

/**
 * A server where schedules run, over whatever `fakeApi` answers: the person's chat in which the
 * model asks to repeat their email summary every weekday at 8, the server's card waiting for
 * them, recorded as the server records it; "Repeat it" makes the schedule and its own chat, which
 * ran last Friday and this morning, or with `outcome` `refused` the server refuses it; "Not now"
 * makes nothing. `existing`, the schedule and its chat are there from the start. The schedule
 * is listed, and pauses, resumes and goes as asked. Each time zone the page gives is kept in `zones`.
 */
export async function schedules(
  page: Page,
  {
    outcome = 'made',
    existing = false,
  }: { readonly outcome?: Outcome; readonly existing?: boolean } = {},
): Promise<Kept> {
  const kept: Kept = {
    outcome,
    existing,
    decision: null,
    paused: false,
    removed: false,
    zones: [],
  };
  await answering(page, CARD, kept);
  return kept;
}
