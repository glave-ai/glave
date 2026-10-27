import type { ChangedFile, FileChange, RawEvent, ThreadChanges } from '@glave/client';

/** What the fake server says changes leave out, as the server does. */
const NOTE = 'Changes made by commands or other tools are not listed.';

type Json = Readonly<Record<string, unknown>>;

function linesOf(text: unknown): readonly string[] {
  return typeof text === 'string' && text !== '' ? text.replace(/\n$/u, '').split('\n') : [];
}

/** The change a call asks for, as the fake reads it: a write puts every line, an edit swaps. */
function changeOf(tool: unknown, json: Json): FileChange | null {
  const added = (text: string) => ({ kind: 'added', text }) as const;
  if (tool === 'write') {
    return { kind: 'wrote', lines: linesOf(json['content']).map(added) };
  }
  if (tool === 'edit') {
    const removed = linesOf(json['old_string']).map((text) => ({ kind: 'removed', text }) as const);
    return { kind: 'edited', lines: [...removed, ...linesOf(json['new_string']).map(added)] };
  }
  return null;
}

/** `files` with `change` to the file at `path` added. */
function adding(files: ChangedFile[], path: string, change: FileChange): void {
  const count = (kind: string): number => change.lines.filter((line) => line.kind === kind).length;
  const file = files.find((each) => each.path === path);
  const added = (file?.added ?? 0) + count('added');
  const removed = (file?.removed ?? 0) + count('removed');
  const changes = [...(file?.changes ?? []), change];
  const at = file === undefined ? files.length : files.indexOf(file);
  files.splice(at, file === undefined ? 0 : 1, { path, added, removed, changes });
}

/**
 * What the fake server says `events` changed, as the server reads a thread's log: each `write`
 * and `edit` that finished without an error, by file and by the turn it was in.
 */
export function changesOf(events: readonly RawEvent[]): ThreadChanges {
  const asked = new Map<unknown, { readonly path: string; readonly change: FileChange }>();
  const files: ChangedFile[] = [];
  const turns: { readonly turn: number; readonly files: ChangedFile[] }[] = [];
  for (const event of events) {
    if (event.type === 'turn.started') {
      turns.push({ turn: event.seq, files: [] });
    }
    const args = event.data['arguments'] as { readonly json?: Json } | undefined;
    const json = args?.json ?? {};
    const change = event.type === 'tool.call.requested' ? changeOf(event.data['tool'], json) : null;
    if (change !== null && typeof json['path'] === 'string') {
      asked.set(event.data['call_id'], { path: json['path'], change });
    }
    const done =
      event.type === 'tool.call.completed' ? asked.get(event.data['call_id']) : undefined;
    if (done !== undefined && event.data['is_error'] !== true) {
      adding(files, done.path, done.change);
      const turn = turns.at(-1);
      if (turn !== undefined) {
        adding(turn.files, done.path, done.change);
      }
    }
  }
  return { files, turns: turns.filter((turn) => turn.files.length > 0), note: NOTE };
}
