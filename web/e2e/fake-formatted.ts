import type { ChangedFile, RunningCommand, ThreadSummary } from '@glave/client';

import { type Answers, WORKSPACE, call, done, event } from './fake-records';

/**
 * A thread in a repository that changed files only by a command: it ran the formatter, then
 * left the tests watching. Its turns edited nothing of glave's own, so what it changed is known
 * only once its folder says.
 */
export const FORMATTED = '01900000-0000-7000-8000-000000000151';

/** What it keeps running in the background. */
const WATCHING: RunningCommand = {
  id: '01900000-0000-7000-8000-0000000000d3',
  command: 'cargo watch -x test',
  started_at: '2026-10-11T10:02:00.000Z',
  summary: 'Still running: the agent started it, and it runs until it ends or someone stops it.',
};

/** Where it works: the project's folder, a repository, read since the commit it started at. */
const SINCE = { ...WORKSPACE, since: '4f6c2a9e1d3b8c7a5e0f2d4b6a8c1e3f5a7b9d0c' };

/** What its summary says beyond any thread's: it works in a repository. */
export const FORMATTED_SUMMARY: Partial<ThreadSummary> = { workspace: SINCE };

/** Its history: the formatter run, then the tests left watching. */
export const FORMATTED_HISTORY = [
  event(1, 'thread.started', {}, FORMATTED),
  event(2, 'workspace.bound', { ...SINCE }, FORMATTED),
  event(3, 'turn.started', {}, FORMATTED),
  event(
    4,
    'message.added',
    {
      role: 'user',
      parts: [{ type: 'text', text: 'Format the code, then keep the tests running' }],
    },
    FORMATTED,
  ),
  call(5, 'call_1', 'bash', { command: 'cargo fmt' }, FORMATTED),
  done(6, 'call_1', '', FORMATTED),
  call(7, 'call_2', 'bash', { command: WATCHING.command, background: true }, FORMATTED),
  done(8, 'call_2', 'Running in the background as command 1.', FORMATTED),
  event(
    9,
    'command.started',
    { command: WATCHING.id, call_id: 'call_2', script: WATCHING.command },
    FORMATTED,
  ),
  event(
    10,
    'message.added',
    {
      role: 'assistant',
      parts: [{ type: 'text', text: 'Formatted; the tests run again on each save.' }],
    },
    FORMATTED,
  ),
  event(11, 'turn.ended', { outcome: 'completed' }, FORMATTED, '2026-10-11T10:02:05.000Z'),
];

/** A file the formatter rewrote, as its folder shows the change: one stretch. */
function formatted(path: string, before: string, after: string): ChangedFile {
  return {
    path,
    added: 1,
    removed: 1,
    changes: [
      {
        kind: 'edited',
        lines: [
          { kind: 'removed', text: before },
          { kind: 'added', text: after },
        ],
      },
    ],
  };
}

/** What its folder says it changed: the files the formatter rewrote. */
const FILES: readonly ChangedFile[] = [
  formatted('src/args.rs', 'pub struct Args{', 'pub struct Args {'),
  formatted('src/main.rs', '    run( args )', '    run(args)'),
];

/** Calls about the formatting thread, whose summary the fake keeps; no list holds it. */
export const ABOUT_FORMATTED: Answers<ThreadSummary> = {
  'thread/get': (params, summary) =>
    params.thread === FORMATTED ? { thread: summary } : undefined,
  'command/list': (params) =>
    params.thread === FORMATTED ? { commands: [WATCHING], previewed: false } : undefined,
  'thread/changes': (params) =>
    params.thread === FORMATTED
      ? {
          changes: {
            files: FILES,
            turns: [],
            note: "The thread's files are glave's own edits and all else that differs from the version it started from, whatever changed it; a turn's are only glave's own edits.",
          },
        }
      : undefined,
};
