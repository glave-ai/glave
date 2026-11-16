import type { RawEvent, ScheduleInfo, ThreadSummary } from '@glave/client';
import type { Page, Route } from '@playwright/test';

import { PERSONAL } from './fake-api';
import { type Answers, SCRATCH, answerFrom, event, posted } from './fake-records';
import { discovered } from './fake-signin';

/** The chat the person asked in to repeat their email summary. */
export const ASKING = '01900000-0000-7000-8000-0000000005c1';
/** The schedule's own chat, which "Repeat it" makes. */
export const MORNING = '01900000-0000-7000-8000-0000000005c2';
/** The schedule "Repeat it" makes. */
const SCHEDULE = '01900000-0000-7000-8000-0000000005c3';
/** The time zone the schedule repeats in, which the tests' browser is in too. */
export const ZONE = 'Europe/Lisbon';

const NONE = { input: 0, output: 0, cache_read: 0, cache_write: 0, reasoning: 0 };

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

/** The schedule, as the server lists it: paused, or running next tomorrow at 8. */
function morning(paused: boolean): ScheduleInfo {
  return {
    schedule: SCHEDULE,
    name: 'Morning email summary',
    project: PERSONAL.id,
    thread: MORNING,
    from: ASKING,
    prompt: 'Summarize my email',
    when: { cron: '0 8 * * 1-5' },
    zone: ZONE,
    repeats: 'Every weekday at 8:00 am',
    budget: 200_000,
    created_at: '2026-11-16T09:01:00.000Z',
    paused,
    ...(paused ? {} : { next: '2026-11-17T08:00:00.000Z' }),
    last: { due: '2026-11-16T08:00:00.000Z', status: 'ended', outcome: 'completed', said: 'Done' },
  };
}

const CALL = 'call_3';

/** What the asking chat records: the person's ask, the model's call, and the server's card. */
const ASKED: readonly RawEvent[] = [
  event(1, 'turn.started', {}, ASKING, '2026-11-16T09:00:00.000Z'),
  event(
    2,
    'message.added',
    {
      role: 'user',
      parts: [{ type: 'text', text: 'Summarize my email every weekday at 8.' }],
    },
    ASKING,
  ),
  event(
    3,
    'tool.call.requested',
    {
      call_id: CALL,
      tool: 'schedule',
      arguments: { json: { action: 'create', name: 'Morning email summary' } },
      summary: 'schedule: Morning email summary',
    },
    ASKING,
  ),
  event(
    4,
    'approval.requested',
    {
      call_id: CALL,
      tool: 'schedule',
      summary: 'schedule: Morning email summary',
      ask: 'Repeat this every weekday at 8:00 am?',
      asking: { says: 'ask-repeat', when: 'every weekday at 8:00 am' },
      detail:
        'First run: Tuesday 17 November at 8:00 am, your time.\nEach run is added to its own chat, “Morning email summary”.',
      caution: 'It runs only while glave runs on this computer.',
    },
    ASKING,
  ),
];

/** What it records once the person says Repeat it: the schedule made, and the model's words. */
const MADE: readonly RawEvent[] = [
  event(5, 'approval.decided', { call_id: CALL, decision: 'allow_once' }, ASKING),
  event(
    6,
    'tool.call.completed',
    {
      call_id: CALL,
      is_error: false,
      output: [{ type: 'text', text: 'Made the schedule.' }],
      structured: { schedule: SCHEDULE, chat: MORNING },
    },
    ASKING,
  ),
  event(
    7,
    'message.added',
    {
      role: 'assistant',
      parts: [{ type: 'text', text: 'Done: your summary comes every weekday at 8.' }],
    },
    ASKING,
  ),
  event(8, 'turn.ended', { outcome: 'completed' }, ASKING),
];

/** The schedule's chat: this morning's run, headed with when it was due. */
const RAN: readonly RawEvent[] = [
  event(1, 'turn.started', { scheduled: '2026-11-16T08:00:00.000Z' }, MORNING),
  event(
    2,
    'message.added',
    {
      role: 'user',
      parts: [{ type: 'text', text: 'Summarize my email' }],
    },
    MORNING,
  ),
  event(
    3,
    'message.added',
    {
      role: 'assistant',
      parts: [{ type: 'text', text: 'Three need a reply today.' }],
    },
    MORNING,
  ),
  event(4, 'turn.ended', { outcome: 'completed' }, MORNING),
];

/** What the fake keeps while it runs. */
interface Kept {
  /** Whether the person said Repeat it. */
  made: boolean;
  paused: boolean;
  /** Each time zone the page gave, in order. */
  readonly zones: string[];
}

function eventsOf(thread: string, kept: Kept): readonly RawEvent[] {
  if (thread === MORNING) {
    return RAN;
  }
  return thread === ASKING ? [...ASKED, ...(kept.made ? MADE : [])] : [];
}

function summaryOf(thread: string, kept: Kept): ThreadSummary | undefined {
  if (thread === MORNING && kept.made) {
    return chat(MORNING, 'Morning email summary', { schedule: morning(kept.paused) });
  }
  return thread === ASKING ? chat(ASKING, 'Summarize my email') : undefined;
}

const SCHEDULES: Answers<Kept> = {
  'server/discover': () => ({ ...discovered(), schedules: true }),
  'zone/set': ({ zone }, kept) => {
    kept.zones.push(zone);
    return {};
  },
  'thread/list': ({ project }, kept) => {
    const morningChat = summaryOf(MORNING, kept);
    const asking = chat(ASKING, 'Summarize my email');
    return project === PERSONAL.id
      ? { threads: morningChat === undefined ? [asking] : [morningChat, asking] }
      : undefined;
  },
  'thread/get': ({ thread }, kept) => {
    const summary = summaryOf(thread, kept);
    return summary === undefined ? undefined : { thread: summary };
  },
  'thread/turns': ({ thread }, kept) =>
    summaryOf(thread, kept) === undefined ? undefined : { events: eventsOf(thread, kept) },
  'thread/events': ({ thread }, kept) =>
    summaryOf(thread, kept) === undefined ? undefined : { events: eventsOf(thread, kept) },
  'thread/steps': ({ thread }, kept) => {
    if (thread === ASKING) {
      const runs = [{ first: 3, calls: [CALL], sentence: 'Set up a schedule' }];
      return { steps: { runs, replies: kept.made ? [7] : [] } };
    }
    return thread === MORNING ? { steps: { runs: [], replies: [3] } } : undefined;
  },
  'approval/decide': ({ thread, decision }, kept) => {
    if (thread !== ASKING) {
      return undefined;
    }
    kept.made = decision === 'allow_once';
    return {};
  },
  'schedule/list': (_, kept) => ({ schedules: kept.made ? [morning(kept.paused)] : [] }),
  'schedule/pause': ({ paused }, kept) => {
    kept.paused = paused;
    return { schedule: morning(paused) };
  },
};

/**
 * A server where schedules run, over whatever `fakeApi` answers: the person's chat in which the
 * model asks to repeat their email summary every weekday at 8, the server's card waiting for
 * them; "Repeat it" makes the schedule and its own chat, which ran this morning; and the
 * schedule listed, paused and resumed. Each time zone the page gives is kept in `zones`.
 */
export async function schedules(page: Page): Promise<{ readonly zones: readonly string[] }> {
  const kept: Kept = { made: false, paused: false, zones: [] };
  await page.route('**/*', async (route: Route) => {
    const message = await posted(route);
    if (message === null) {
      return;
    }
    const result = answerFrom(SCHEDULES, message.method, message.params, kept);
    if (result === undefined) {
      await route.fallback();
      return;
    }
    await route.fulfill({
      contentType: 'application/json',
      body: JSON.stringify({ jsonrpc: '2.0', id: message.id, result }),
    });
  });
  return { zones: kept.zones };
}
