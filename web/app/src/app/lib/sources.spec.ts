import type { RawEvent } from '@glave/client';

import { event } from '../testing/fixtures';
import { lastSourcing, shortAddress, sourceLinks } from './sources';

function requested(seq: number, tool: string): RawEvent {
  return event(seq, 'tool.call.requested', {
    call_id: `c${String(seq)}`,
    tool,
    arguments: { json: {} },
  });
}

describe('lastSourcing', () => {
  it('is the end of the latest turn once the thread asked to read a page', () => {
    const events = [
      event(1, 'turn.started', {}),
      event(2, 'turn.ended', { outcome: 'completed' }),
      event(3, 'turn.started', {}),
      requested(4, 'web_fetch'),
      event(5, 'turn.ended', { outcome: 'completed' }),
      event(6, 'turn.started', {}),
      event(7, 'turn.ended', { outcome: 'completed' }),
    ];
    expect(lastSourcing(events.slice(0, 2))).toBeNull();
    expect(lastSourcing(events.slice(0, 4))).toBeNull();
    expect(lastSourcing(events.slice(0, 5))).toBe(5);
    expect(lastSourcing(events)).toBe(7);
  });

  it('asks nothing after other tools, a search among them', () => {
    const events = [
      requested(1, 'mcp__search__web_search_exa'),
      requested(2, 'read'),
      event(3, 'turn.ended', { outcome: 'completed' }),
    ];
    expect(lastSourcing(events)).toBeNull();
  });
});

describe('sourceLinks', () => {
  it('links each page by its address, titled when it gave a title', () => {
    const links = sourceLinks({
      reply: 9,
      sentence: 'Sources: 2 pages',
      pages: [
        { url: 'https://tokio.rs/blog/2026-10', title: 'Tokio blog' },
        { url: 'https://docs.rs/', title: null },
      ],
    });
    expect(links).toEqual([
      {
        href: 'https://tokio.rs/blog/2026-10',
        title: 'Tokio blog',
        address: 'tokio.rs/blog/2026-10',
      },
      { href: 'https://docs.rs/', title: null, address: 'docs.rs' },
    ]);
  });

  it('shows an address it cannot read as it is', () => {
    expect(shortAddress('not a url')).toBe('not a url');
    expect(shortAddress('http://127.0.0.1:8080/a?b=1')).toBe('127.0.0.1:8080/a?b=1');
  });
});
