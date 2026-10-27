import type { Page } from '@playwright/test';

import { HISTORY } from './fake-api';
import { changesOf } from './fake-changes';
import { type Storing, storedEvents } from './fake-events';
import { type Gate, type Gated, gates } from './fake-gate';
import { THREAD, call, done, event } from './fake-records';

/** What the person asks in a turn that thinks first (`thinkingTurn`). */
export const THINKING_PROMPT = 'Why does the test hang?';
/** When the server records that turn's start. */
export const STARTED = '2026-10-11T10:10:00.000Z';
/** The step its tool takes, in the server's words: it writes the file the turn changes. */
export const STEP = 'write src/lock.rs (2 lines)';
/** What the model thinks, in the pieces it streams, ending in a blank line as a model's may. */
export const THINKING = ['The lock is taken ', 'twice on the same thread.\n\n'];
/** What it answers, in the pieces it streams. */
export const ANSWER = ['The lock ', 'is taken twice.'];

/** The file the turn writes, and what the server says the thread changed once it has. */
const LOCK = { path: 'src/lock.rs', content: 'one\ntwo\n' };
const WROTE = changesOf([
  ...HISTORY,
  event(HISTORY.length + 1, 'turn.started', {}),
  call(HISTORY.length + 3, 'call_test', 'write', LOCK),
  done(HISTORY.length + 4, 'call_test', 'Wrote src/lock.rs'),
]);

/**
 * Makes the page's next turn in `THREAD` one that runs a tool, then thinks before it answers, a
 * step at a time, each at a gate the test lets go (`letGo`), in order: `start` records the turn
 * (at `STARTED`) and the person's message, `tool` writes `src/lock.rs` (the server saying `STEP`
 * as it begins), `think` ends it and streams the model's reasoning, and `answer` streams its
 * reply, records it with its reasoning, and ends the turn. Until `start`, the server has
 * recorded nothing, as when a model call is slow to begin. The page's own `fetch` answers, as in
 * `streamReply`.
 */
export async function thinkingTurn(page: Page): Promise<void> {
  await gates(page);
  await storedEvents(page);
  await page.addInitScript(
    ({ after, thread, started, step, thinking, answer, lock, wrote }) => {
      const real = window.fetch.bind(window);
      const encoder = new TextEncoder();
      const passed = (gate: Gate): Promise<void> =>
        (window as unknown as Gated).glaveGates.passed(gate);
      const stored = (seq: number, type: string, data: unknown): string =>
        (window as unknown as Storing).glaveStored(thread, seq, started, type, data);
      const live = (update: unknown): string =>
        JSON.stringify({ jsonrpc: '2.0', method: 'turn/live', params: { thread, update } });
      const call = { call_id: 'call_test' };
      // Once the file is written, the server says so when asked what the thread changed.
      let written = false;
      window.fetch = async (input, init) => {
        const body = typeof init?.body === 'string' ? init.body : '';
        if (written && body.includes('"thread/changes"')) {
          const { id } = JSON.parse(body) as { id: number };
          const result = JSON.stringify({ jsonrpc: '2.0', id, result: { changes: wrote } });
          return new Response(result, { headers: { 'Content-Type': 'application/json' } });
        }
        if (!body.includes('"turn/start"')) {
          return real(input, init);
        }
        const { id, params } = JSON.parse(body) as { id: number; params: { prompt: string } };
        const stream = new ReadableStream<Uint8Array>({
          async start(controller) {
            const send = (message: string): void => {
              controller.enqueue(encoder.encode(`data: ${message}\n\n`));
            };
            await passed('start');
            send(stored(after + 1, 'turn.started', {}));
            send(
              stored(after + 2, 'message.added', {
                role: 'user',
                parts: [{ type: 'text', text: params.prompt }],
              }),
            );
            send(live({ kind: 'waiting' }));
            await passed('tool');
            send(
              stored(after + 3, 'tool.call.requested', {
                ...call,
                tool: 'write',
                arguments: { json: lock },
                summary: step,
              }),
            );
            send(live({ kind: 'tool_started', summary: step }));
            await passed('think');
            send(live({ kind: 'tool_finished', summary: step, is_error: false }));
            written = true;
            send(
              stored(after + 4, 'tool.call.completed', {
                ...call,
                is_error: false,
                // Ending in a blank line, as a command's output may.
                output: [{ type: 'text', text: 'Wrote src/lock.rs\n\n' }],
              }),
            );
            for (const piece of thinking) {
              send(live({ kind: 'reasoning', text: piece }));
            }
            await passed('answer');
            for (const piece of answer) {
              send(live({ kind: 'text', text: piece }));
            }
            send(
              stored(after + 5, 'message.added', {
                role: 'assistant',
                parts: [
                  { type: 'reasoning', text: thinking.join('') },
                  { type: 'text', text: answer.join('') },
                ],
              }),
            );
            send(stored(after + 6, 'turn.ended', { outcome: 'completed' }));
            const tokens = { input: 1, output: 1, cache_read: 0, cache_write: 0, reasoning: 1 };
            const turn = { outcome: 'completed', model_calls: 2, tool_calls: 1, tokens };
            send(JSON.stringify({ jsonrpc: '2.0', id, result: { turn } }));
            controller.close();
          },
        });
        return new Response(stream, { headers: { 'Content-Type': 'text/event-stream' } });
      };
    },
    {
      after: HISTORY.length,
      thread: THREAD,
      started: STARTED,
      step: STEP,
      thinking: THINKING,
      answer: ANSWER,
      lock: LOCK,
      wrote: WROTE,
    },
  );
}
