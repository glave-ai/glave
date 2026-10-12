import type { RawEvent } from '@glave/client';

import { event } from '../testing/fixtures';
import { changedFiles, editLines } from './files';

function call(seq: number, id: string, tool: string, json: Record<string, unknown>): RawEvent {
  return event(seq, 'tool.call.requested', { call_id: id, tool, arguments: { json } });
}

function done(seq: number, id: string, isError = false): RawEvent {
  return event(seq, 'tool.call.completed', { call_id: id, is_error: isError, output: [] });
}

describe('editLines', () => {
  it('keeps the lines both share at the ends, and swaps the rest', () => {
    expect(editLines('a\nb\nc\n', 'a\nB\nc\n')).toEqual([
      { kind: 'same', sign: ' ', text: 'a' },
      { kind: 'removed', sign: '-', text: 'b' },
      { kind: 'added', sign: '+', text: 'B' },
      { kind: 'same', sign: ' ', text: 'c' },
    ]);
  });

  it('handles pieces that only add, only remove, or repeat', () => {
    expect(editLines('', 'new')).toEqual([{ kind: 'added', sign: '+', text: 'new' }]);
    expect(editLines('gone\r\n', '')).toEqual([{ kind: 'removed', sign: '-', text: 'gone' }]);
    expect(editLines('x\nx', 'x\nx\nx').map((each) => each.kind)).toEqual([
      'same',
      'same',
      'added',
    ]);
  });
});

describe('changedFiles', () => {
  it('groups the successful writes and edits by file, in the order first changed', () => {
    const files = changedFiles([
      call(1, 'c1', 'write', { path: 'src/new.ts', content: 'one\ntwo\n' }),
      done(2, 'c1'),
      call(3, 'c2', 'edit', { path: 'README.md', old_string: 'Old', new_string: 'New' }),
      done(4, 'c2'),
      call(5, 'c3', 'edit', {
        path: 'src/new.ts',
        old_string: 'two',
        new_string: 'three',
        replace_all: true,
      }),
      done(6, 'c3'),
    ]);
    expect(files.map((file) => [file.path, file.added, file.removed])).toEqual([
      ['src/new.ts', 3, 1],
      ['README.md', 1, 1],
    ]);
    expect(files[0]?.changes.map((change) => [change.key, change.tool, change.all])).toEqual([
      ['c1', 'write', false],
      ['c3', 'edit', true],
    ]);
  });

  it('leaves out failed and unfinished calls, other tools and malformed arguments', () => {
    expect(
      changedFiles([
        call(1, 'c1', 'edit', { path: 'a.ts', old_string: 'x', new_string: 'y' }),
        done(2, 'c1', true),
        call(3, 'c2', 'write', { path: 'b.ts', content: 'x' }),
        call(4, 'c3', 'bash', { command: 'echo > c.ts' }),
        done(5, 'c3'),
        call(6, 'c4', 'write', { content: 'no path' }),
        done(7, 'c4'),
        event(8, 'tool.call.requested', {
          call_id: 'c5',
          tool: 'write',
          arguments: { malformed: '{' },
        }),
        done(9, 'c5'),
        done(10, 'unknown'),
      ]),
    ).toEqual([]);
  });
});
