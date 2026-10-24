import type {
  ModelInfo,
  RawEvent,
  RunningCommand,
  ScheduleInfo,
  Project,
  TaskInfo,
  ThreadSummary,
  TokenCounts,
} from '@glave/client';

/** Test data shaped as the session API sends it. */

export const TOKENS: TokenCounts = {
  input: 1200,
  output: 300,
  cache_read: 0,
  cache_write: 0,
  reasoning: 0,
};

export function record(id: string, name: string, kind: Project['kind'] = 'project'): Project {
  return {
    id,
    kind,
    name,
    owner: { kind: 'person', id: 'person-1' },
    created_at: '2026-10-11T10:00:00.000Z',
    updated_at: '2026-10-11T10:00:00.000Z',
  };
}

export function thread(
  id: string,
  title: string | null,
  overrides: Partial<ThreadSummary> = {},
): ThreadSummary {
  return {
    id,
    project: 'project-1',
    status: 'idle',
    standing: 'active',
    title,
    approval: 'auto',
    origin: { kind: 'web' },
    created_at: '2026-10-11T10:00:00.000Z',
    updated_at: '2026-10-11T10:05:00.000Z',
    turns: 1,
    tokens: TOKENS,
    ...overrides,
  };
}

export function event(
  seq: number,
  type: string,
  data: Readonly<Record<string, unknown>>,
  threadId = 'thread-1',
): RawEvent {
  return {
    glave: 1,
    id: `event-${String(seq)}`,
    thread: threadId,
    seq,
    at: '2026-10-11T10:00:00.000Z',
    type,
    v: 1,
    data,
  };
}

export function task(threadId: string, title: string, overrides: Partial<TaskInfo> = {}): TaskInfo {
  return {
    thread: threadId,
    project: 'project-1',
    kind: 'agent',
    title,
    model: 'hope',
    tokens: TOKENS,
    cost: { micros: 4200, currency: 'USD' },
    started_at: '2026-10-11T10:00:00.000Z',
    elapsed_ms: 125_000,
    status: 'running',
    ...overrides,
  };
}

/** A command running in the background, as `command/list` lists one, started at 10:00. */
export function command(
  id: string,
  script: string,
  overrides: Partial<RunningCommand> = {},
): RunningCommand {
  return {
    id,
    command: script,
    started_at: '2026-10-11T10:00:00.000Z',
    summary: 'Still running: the agent started it, and it runs until it ends or someone stops it.',
    ...overrides,
  };
}

/** A schedule, with its next run or none (a one-off that has run). */
export function schedule(id: string, next: string | null): ScheduleInfo {
  return {
    schedule: id,
    prompt: 'Audit the dependencies',
    when: { cron: '0 2 * * *' },
    budget: 100_000,
    created_at: '2026-10-11T10:00:00.000Z',
    next,
  };
}

/** A server's models: the default, and Hope. */
export const MODELS: readonly ModelInfo[] = [
  { id: 'gpt-6-luna', default: true },
  { id: 'hope-12b', name: 'Hope', default: false },
];

/** What `server/discover` says. */
export const SERVER = {
  name: 'glave',
  version: '0.1.0',
  protocols: ['2026-10'],
  approval: 'auto',
  approvals: ['ask', 'auto'],
} as const;
