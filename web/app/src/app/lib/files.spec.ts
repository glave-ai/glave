import type { RawEvent } from '@glave/client';

import { event } from '../testing/fixtures';
import { lastChanging, lineCounts } from './files';

function call(seq: number, id: string, tool: string): RawEvent {
  return event(seq, 'tool.call.requested', { call_id: id, tool, arguments: { json: {} } });
}

function done(seq: number, id: string, isError = false): RawEvent {
  return event(seq, 'tool.call.completed', { call_id: id, is_error: isError, output: [] });
}

describe('lastChanging', () => {
  it('is the latest end of a call that can change files, whatever came of it', () => {
    expect(lastChanging([])).toBeNull();
    expect(lastChanging([call(1, 'c1', 'write')])).toBeNull();
    expect(
      lastChanging([
        call(1, 'c1', 'edit'),
        done(2, 'c1', true),
        call(3, 'c2', 'agent'),
        done(4, 'c2'),
        call(5, 'c3', 'bash'),
        done(6, 'c3'),
        call(7, 'c4', 'read'),
        done(8, 'c4'),
      ]),
    ).toBe(4);
  });
});

describe('lineCounts', () => {
  it('marks lines added and removed, and says them in words', () => {
    expect(lineCounts(1_200, 1, 'en-US')).toEqual({
      added: '+1,200',
      removed: '−1',
      label: '1,200 lines added, 1 removed',
    });
  });
});
