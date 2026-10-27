import type { Page } from '@playwright/test';

import { HISTORY } from './fake-api';
import { type Gate, type Gated, gates } from './fake-gate';
import { THREAD } from './fake-records';

/** The id the fake answers a message sent while its turn runs with (`steerTurn`). */
const QUEUED_ID = '01900000-0000-7000-8000-000000000999';

/** What a turn that is steered sends: `steerTurn`'s page script. */
interface Steering {
  /** The person's message, recorded at `seq`, under `id` when it has one. */
  readonly said: (seq: number, text: string, id?: string) => string;
}

/**
 * Makes the page's next turn in `THREAD` one the person steers: it starts on their message and
 * runs a command, then waits at the gate `read` until the test lets the model read (`letGo`).
 * A message sent meanwhile is answered at once as queued; once read, it is recorded where the
 * model saw it, and the model answers it. The page's own `fetch` answers, as in `streamReply`.
 */
export async function steerTurn(page: Page): Promise<void> {
  const after = HISTORY.length;
  await gates(page);
  await page.addInitScript(
    ({ after, thread, queuedId }) => {
      const real = window.fetch.bind(window);
      const encoder = new TextEncoder();
      const passed = (gate: Gate): Promise<void> =>
        (window as unknown as Gated).glaveGates.passed(gate);
      let sent = '';
      let turns = 0;
      const at = '2026-10-11T10:10:00.000Z';
      const stored = (seq: number, type: string, data: unknown, id?: string): string =>
        JSON.stringify({
          jsonrpc: '2.0',
          method: 'thread/event',
          params: {
            thread,
            event: {
              glave: 1,
              id: id ?? `01900000-0000-7000-8000-${String(seq).padStart(12, '0')}`,
              thread,
              seq,
              at,
              type,
              v: 1,
              data,
            },
          },
        });
      const steering: Steering = {
        said: (seq, text, id) =>
          stored(seq, 'message.added', { role: 'user', parts: [{ type: 'text', text }] }, id),
      };
      const live = (text: string): string =>
        JSON.stringify({
          jsonrpc: '2.0',
          method: 'turn/live',
          params: { thread, update: { kind: 'text', text } },
        });
      window.fetch = async (input, init) => {
        const body = typeof init?.body === 'string' ? init.body : '';
        if (!body.includes('"turn/start"')) {
          return real(input, init);
        }
        const { id, params } = JSON.parse(body) as { id: number; params: { prompt: string } };
        turns += 1;
        if (turns > 1) {
          sent = params.prompt;
          const queued = { jsonrpc: '2.0', id, result: { queued: { id: queuedId } } };
          return new Response(JSON.stringify(queued), {
            headers: { 'Content-Type': 'application/json' },
          });
        }
        const command = { command: 'sed -i s/8080/3000/ config.toml' };
        const stream = new ReadableStream<Uint8Array>({
          async start(controller) {
            const send = (message: string): void => {
              controller.enqueue(
                encoder.encode(`data: ${message}

`),
              );
            };
            send(stored(after + 1, 'turn.started', {}));
            send(steering.said(after + 2, params.prompt));
            send(live('Setting the port to 3000.'));
            send(
              stored(after + 3, 'tool.call.requested', {
                call_id: 'call_9',
                tool: 'bash',
                arguments: { json: command },
                summary: `bash: ${command.command}`,
              }),
            );
            send(
              stored(after + 4, 'tool.call.completed', {
                call_id: 'call_9',
                is_error: false,
                output: [{ type: 'text', text: '' }],
              }),
            );
            await passed('read');
            send(steering.said(after + 5, sent, queuedId));
            send(live('Using 4000 instead.'));
            send(
              stored(after + 6, 'message.added', {
                role: 'assistant',
                parts: [{ type: 'text', text: 'Using 4000 instead.' }],
              }),
            );
            send(stored(after + 7, 'turn.ended', { outcome: 'completed' }));
            const tokens = { input: 1, output: 1, cache_read: 0, cache_write: 0, reasoning: 0 };
            const turn = { outcome: 'completed', model_calls: 2, tool_calls: 1, tokens };
            send(JSON.stringify({ jsonrpc: '2.0', id, result: { turn } }));
            controller.close();
          },
        });
        return new Response(stream, { headers: { 'Content-Type': 'text/event-stream' } });
      };
    },
    { after, thread: THREAD, queuedId: QUEUED_ID },
  );
}
