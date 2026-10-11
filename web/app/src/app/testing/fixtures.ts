import type { RawEvent, Space, TaskInfo, ThreadSummary, TokenCounts } from '@glave/client';

/** Test data shaped as the session API sends it. */

export const TOKENS: TokenCounts = {
  input: 1200,
  output: 300,
  cache_read: 0,
  cache_write: 0,
  reasoning: 0,
};

export function space(id: string, name: string, kind: Space['kind'] = 'project'): Space {
  return {
    id,
    kind,
    name,
    owner: { kind: 'person', id: 'person-1' },
    members: [],
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
    space: 'space-1',
    status: 'idle',
    standing: 'active',
    title,
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
