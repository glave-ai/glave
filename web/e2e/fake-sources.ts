import type { ReplySources } from '@glave/client';

import { call, done, event } from './fake-records';

/** A chat that searched the web and read what it found, then answered from it. */
export const RESEARCH = '01900000-0000-7000-8000-000000000111';

/** The pages the chat read, as `web_fetch` recorded them. */
export const PAGES = [
  { url: 'https://tokio.rs/blog/2026-10-tokio-2', title: 'Announcing Tokio 2.0' },
  { url: 'https://docs.rs/tokio/latest/tokio/', title: null },
] as const;

/** A `web_fetch` that read `page`, as its `tool.call.completed` records it. */
function read(seq: number, callId: string, page: (typeof PAGES)[number]): ReturnType<typeof done> {
  const completed = done(seq, callId, `URL: ${page.url}\n\nThe page.`, RESEARCH);
  return { ...completed, data: { ...completed.data, untrusted: true, structured: { page } } };
}

/** The chat's history: a search, a page it read, one that failed to load, another it read. */
export const RESEARCH_HISTORY = [
  event(1, 'thread.started', {}, RESEARCH),
  event(2, 'turn.started', {}, RESEARCH),
  event(
    3,
    'message.added',
    { role: 'user', parts: [{ type: 'text', text: 'What changed in Tokio 2.0?' }] },
    RESEARCH,
  ),
  call(4, 'call_1', 'mcp__search__web_search_exa', { query: 'Tokio 2.0 release' }, RESEARCH),
  done(
    5,
    'call_1',
    'Title: Announcing Tokio 2.0\nURL: https://tokio.rs/blog/2026-10-tokio-2',
    RESEARCH,
  ),
  call(6, 'call_2', 'web_fetch', { url: PAGES[0].url }, RESEARCH),
  read(7, 'call_2', PAGES[0]),
  call(8, 'call_3', 'web_fetch', { url: 'https://tokio.rs/missing' }, RESEARCH),
  event(
    9,
    'tool.call.completed',
    {
      call_id: 'call_3',
      is_error: true,
      output: [{ type: 'text', text: 'Not fetched: the server answered 404 Not Found.' }],
    },
    RESEARCH,
  ),
  call(10, 'call_4', 'web_fetch', { url: PAGES[1].url }, RESEARCH),
  read(11, 'call_4', PAGES[1]),
  event(
    12,
    'message.added',
    {
      role: 'assistant',
      parts: [
        {
          type: 'text',
          text: 'Tokio 2.0 drops the old `io` traits for the standard library’s, and its runtime starts faster.',
        },
      ],
    },
    RESEARCH,
  ),
  event(13, 'turn.ended', { outcome: 'completed' }, RESEARCH, '2026-10-11T10:00:09.000Z'),
];

/** What the chat's reply drew on, as the server reads it from the history. */
export const RESEARCH_SOURCES: ReplySources = {
  reply: 12,
  sentence: 'Sources: 2 pages',
  pages: [...PAGES],
};

/** The answer to `thread/sources`: the research chat's reply drew on two pages; others, none. */
export function sourcesOf(params: Record<string, unknown>): {
  readonly sources: readonly ReplySources[];
} {
  return { sources: params['thread'] === RESEARCH ? [RESEARCH_SOURCES] : [] };
}
