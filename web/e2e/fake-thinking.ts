import type { Page } from '@playwright/test';

import { HISTORY } from './fake-api';
import { THREAD } from './fake-records';

/** What the person asks in a turn that thinks first (`thinkingTurn`). */
export const THINKING_PROMPT = 'Why does the test hang?';
/** What the model thinks, in the pieces it streams. */
export const THINKING = ['The lock is taken ', 'twice on the same thread.'];
/** What it answers, in the pieces it streams. */
export const ANSWER = ['The lock ', 'is taken twice.'];

/** The steps of a thinking turn the test lets happen, one at a time, in order. */
export type ThinkingStep = 'start' | 'think' | 'answer';

/**
 * Makes the page's next turn in `THREAD` one that thinks before it answers, a step at a time as
 * the test lets it (`letThink`): `start` records the turn and the person's message, `think`
 * streams the model's reasoning, and `answer` streams its reply, records it with its
 * reasoning, and ends the turn. Until `start`, the server has recorded nothing, as when a model
 * call is slow to begin. The page's own `fetch` answers, as in `streamReply`.
 */
export async function thinkingTurn(page: Page): Promise<void> {
  await page.addInitScript(
    ({ after, thread, thinking, answer }) => {
      const real = window.fetch.bind(window);
      const encoder = new TextEncoder();
      const gates = new Map<string, () => void>();
      const passed = new Map<string, Promise<void>>();
      for (const step of ['start', 'think', 'answer']) {
        passed.set(
          step,
          new Promise<void>((resolve) => {
            gates.set(step, resolve);
          }),
        );
      }
      Object.assign(window, { glaveThinking: (step: string) => gates.get(step)?.() });
      const at = '2026-10-11T10:10:00.000Z';
      const stored = (seq: number, type: string, data: unknown): string =>
        JSON.stringify({
          jsonrpc: '2.0',
          method: 'thread/event',
          params: {
            thread,
            event: {
              glave: 1,
              id: `01900000-0000-7000-8000-${String(seq).padStart(12, '0')}`,
              thread,
              seq,
              at,
              type,
              v: 1,
              data,
            },
          },
        });
      const live = (kind: string, text: string): string =>
        JSON.stringify({
          jsonrpc: '2.0',
          method: 'turn/live',
          params: { thread, update: { kind, text } },
        });
      window.fetch = async (input, init) => {
        const body = typeof init?.body === 'string' ? init.body : '';
        if (!body.includes('"turn/start"')) {
          return real(input, init);
        }
        const { id, params } = JSON.parse(body) as { id: number; params: { prompt: string } };
        const stream = new ReadableStream<Uint8Array>({
          async start(controller) {
            const send = (message: string): void => {
              controller.enqueue(encoder.encode(`data: ${message}\n\n`));
            };
            await passed.get('start');
            send(stored(after + 1, 'turn.started', {}));
            send(
              stored(after + 2, 'message.added', {
                role: 'user',
                parts: [{ type: 'text', text: params.prompt }],
              }),
            );
            send(
              JSON.stringify({
                jsonrpc: '2.0',
                method: 'turn/live',
                params: { thread, update: { kind: 'waiting' } },
              }),
            );
            await passed.get('think');
            for (const piece of thinking) {
              send(live('reasoning', piece));
            }
            await passed.get('answer');
            for (const piece of answer) {
              send(live('text', piece));
            }
            send(
              stored(after + 3, 'message.added', {
                role: 'assistant',
                parts: [
                  { type: 'reasoning', text: thinking.join('') },
                  { type: 'text', text: answer.join('') },
                ],
              }),
            );
            send(stored(after + 4, 'turn.ended', { outcome: 'completed' }));
            const tokens = { input: 1, output: 1, cache_read: 0, cache_write: 0, reasoning: 1 };
            const turn = { outcome: 'completed', model_calls: 1, tool_calls: 0, tokens };
            send(JSON.stringify({ jsonrpc: '2.0', id, result: { turn } }));
            controller.close();
          },
        });
        return new Response(stream, { headers: { 'Content-Type': 'text/event-stream' } });
      };
    },
    { after: HISTORY.length, thread: THREAD, thinking: THINKING, answer: ANSWER },
  );
}

/** Lets the thinking turn take its next `step`. */
export async function letThink(page: Page, step: ThinkingStep): Promise<void> {
  await page.evaluate((step) => {
    (window as unknown as { glaveThinking: (step: string) => void }).glaveThinking(step);
  }, step);
}
