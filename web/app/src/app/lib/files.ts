import type { RawEvent } from '@glave/client';

/** One line of a change: kept, taken out or put in. */
export interface DiffLine {
  readonly kind: 'same' | 'removed' | 'added';
  /** The mark that says which, so colour is never the only sign: ` `, `-` or `+`. */
  readonly sign: string;
  readonly text: string;
}

/** One successful `write` or `edit` of a file, as its call asked for it. */
export interface FileChange {
  /** The call's id. */
  readonly key: string;
  /** `write` puts the whole file; `edit` swaps one piece for another (each piece, with `all`). */
  readonly tool: 'write' | 'edit';
  readonly all: boolean;
  readonly lines: readonly DiffLine[];
}

/** A file the thread changed, with its changes in order and what they added and removed. */
export interface ChangedFile {
  readonly path: string;
  readonly changes: readonly FileChange[];
  readonly added: number;
  readonly removed: number;
}

type Data = Readonly<Record<string, unknown>>;

function isRecord(value: unknown): value is Data {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

function text(value: unknown): string {
  return typeof value === 'string' ? value : '';
}

function linesOf(value: string): readonly string[] {
  return value === '' ? [] : value.replace(/\r\n/gu, '\n').replace(/\n$/u, '').split('\n');
}

const SIGNS = { same: ' ', removed: '-', added: '+' } as const;

function line(kind: DiffLine['kind'], value: string): DiffLine {
  return { kind, sign: SIGNS[kind], text: value };
}

/**
 * The lines of an edit that swapped `before` for `after`: the lines both share at the start
 * and end kept, the rest taken out and put in.
 */
export function editLines(before: string, after: string): readonly DiffLine[] {
  const old = linesOf(before);
  const next = linesOf(after);
  let start = 0;
  while (start < old.length && start < next.length && old[start] === next[start]) {
    start += 1;
  }
  let end = 0;
  while (
    end < old.length - start &&
    end < next.length - start &&
    old[old.length - 1 - end] === next[next.length - 1 - end]
  ) {
    end += 1;
  }
  return [
    ...old.slice(0, start).map((value) => line('same', value)),
    ...old.slice(start, old.length - end).map((value) => line('removed', value)),
    ...next.slice(start, next.length - end).map((value) => line('added', value)),
    ...old.slice(old.length - end).map((value) => line('same', value)),
  ];
}

/** The change a `write` or `edit` call's arguments ask for, or `null` for any other call. */
function changeOf(
  key: string,
  tool: string,
  args: unknown,
): (FileChange & { path: string }) | null {
  const json = isRecord(args) ? args['json'] : undefined;
  if (!isRecord(json) || text(json['path']) === '') {
    return null;
  }
  const path = text(json['path']);
  if (tool === 'write') {
    const lines = linesOf(text(json['content'])).map((value) => line('added', value));
    return { key, path, tool, all: false, lines };
  }
  if (tool === 'edit') {
    const lines = editLines(text(json['old_string']), text(json['new_string']));
    return { key, path, tool, all: json['replace_all'] === true, lines };
  }
  return null;
}

/**
 * The files a thread changed with its own `write` and `edit` calls, from its events: each
 * call that finished without an error, grouped by path in the order the files were first
 * changed. The diffs are what each call asked for, which the tool applied exactly. Commands
 * (`bash`) and subagents' threads can change files too; their changes are not here.
 */
export function changedFiles(events: readonly RawEvent[]): readonly ChangedFile[] {
  const asked = new Map<string, FileChange & { path: string }>();
  const files = new Map<string, FileChange[]>();
  for (const event of events) {
    const call = text(event.data['call_id']);
    if (event.type === 'tool.call.requested') {
      const change = changeOf(call, text(event.data['tool']), event.data['arguments']);
      if (change !== null) {
        asked.set(call, change);
      }
    } else if (event.type === 'tool.call.completed' && event.data['is_error'] !== true) {
      const change = asked.get(call);
      if (change !== undefined) {
        const { path, ...rest } = change;
        files.set(path, [...(files.get(path) ?? []), rest]);
      }
    }
  }
  return [...files].map(([path, changes]) => {
    const lines = changes.flatMap((change) => change.lines);
    return {
      path,
      changes,
      added: lines.filter((each) => each.kind === 'added').length,
      removed: lines.filter((each) => each.kind === 'removed').length,
    };
  });
}
