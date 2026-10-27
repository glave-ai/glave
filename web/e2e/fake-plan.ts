import type { PlanInfo, PlanStep, RawEvent } from '@glave/client';
import type { Page } from '@playwright/test';

import { type Storing, storedEvents } from './fake-events';
import { type Gated, gates } from './fake-gate';
import { call, done, event } from './fake-records';

/** A thread whose turn in plan mode looked around and proposed a plan. */
export const PLANNING = '01900000-0000-7000-8000-000000000121';

/** The plan it proposed. */
export const PROPOSED: readonly PlanStep[] = [
  { step: 'Read how src/args.rs parses flags', status: 'pending' },
  { step: 'Add --verbose to Args and print each step it takes', status: 'pending' },
  { step: 'Run cargo test', status: 'pending' },
];

/** What the server says of `steps`, proposed or being built. */
export function planOf(steps: readonly PlanStep[], proposed: boolean): PlanInfo {
  const done = steps.filter((step) => step.status === 'completed').length;
  const count = (n: number): string => (n === 1 ? '1 step' : `${String(n)} steps`);
  const sentence = proposed
    ? `Proposed plan: ${count(steps.length)}`
    : `Plan: ${String(done)} of ${count(steps.length)} done`;
  return { steps: [...steps], proposed, sentence };
}

/** The thread's history: in plan mode, a look at the code, a few words, then the plan. */
export const PLANNING_HISTORY: readonly RawEvent[] = [
  event(1, 'thread.started', {}, PLANNING),
  event(2, 'permissions.chosen', { permissions: 'plan' }, PLANNING),
  event(3, 'turn.started', {}, PLANNING),
  event(
    4,
    'message.added',
    { role: 'user', parts: [{ type: 'text', text: 'Add a --verbose flag to the CLI' }] },
    PLANNING,
  ),
  call(5, 'call_1', 'search', { pattern: 'fn parse_args' }, PLANNING),
  done(6, 'call_1', 'src/args.rs:12: pub fn parse_args() -> Args {', PLANNING),
  event(
    7,
    'message.added',
    {
      role: 'assistant',
      parts: [
        {
          type: 'text',
          text: 'Flags are parsed in `src/args.rs`, so `--verbose` goes there. Here is the plan.',
        },
      ],
    },
    PLANNING,
  ),
  call(8, 'call_2', 'update_plan', { plan: PROPOSED }, PLANNING, 'update_plan 0/3 done'),
  event(9, 'plan.updated', { steps: PROPOSED, call_id: 'call_2', proposed: true }, PLANNING),
  done(
    10,
    'call_2',
    'Plan proposed, 3 steps: the person builds it or asks you to change it.',
    PLANNING,
  ),
  event(11, 'turn.ended', { outcome: 'completed' }, PLANNING, '2026-10-11T10:00:07.000Z'),
];

/** What the fake server keeps of the plan: whether it was built. */
export interface Planned {
  built: boolean;
  /** The steps as the person's latest edit left them, else the proposal's. */
  steps: readonly PlanStep[];
}

/** The answer to a call about the plan, kept in `plan`; else `undefined`. */
export function planned(method: string, params: Record<string, unknown>, plan: Planned): unknown {
  if (params['thread'] !== PLANNING) {
    return undefined;
  }
  switch (method) {
    case 'plan/edit': {
      const steps = Array.isArray(params['steps']) ? (params['steps'] as readonly unknown[]) : [];
      plan.steps = steps.map((step) => ({ step: String(step).trim(), status: 'pending' }));
      return { plan: planOf(plan.steps, true) };
    }
    case 'plan/build':
      plan.built = true;
      return {};
    default:
      return undefined;
  }
}

/**
 * The page's own `fetch` as the server answers the plan of `PLANNING`, as `streamReply` does a
 * turn: `thread/plan` from what it has recorded so far; `plan/edit` passed to the fake server,
 * the plan it keeps taken from the answer; and `plan/build` told to the fake server, which runs
 * the thread in Auto from then on, and answered with the turn that builds the plan, which marks
 * the first step done and starts the second, waits at the gate `step` until the test lets it
 * go, then finishes them and says so.
 */
export async function planServer(page: Page): Promise<void> {
  await gates(page);
  await storedEvents(page);
  await page.addInitScript(
    ({ thread, proposed, after }) => {
      const real = window.fetch.bind(window);
      const encoder = new TextEncoder();
      type Step = (typeof proposed)[number];
      let steps: readonly Step[] = proposed;
      let proposing = true;
      const plan = (): unknown => {
        const done = steps.filter((step) => step.status === 'completed').length;
        const count = (n: number): string => (n === 1 ? '1 step' : `${String(n)} steps`);
        const sentence = proposing
          ? `Proposed plan: ${count(steps.length)}`
          : `Plan: ${String(done)} of ${count(steps.length)} done`;
        return { steps, proposed: proposing, sentence };
      };
      const answered = (id: number, result: unknown): Response =>
        new Response(JSON.stringify({ jsonrpc: '2.0', id, result }), {
          headers: { 'Content-Type': 'application/json' },
        });
      // Now, as the server records it, so the work's time reads as it would.
      const stored = (seq: number, type: string, data: unknown): string =>
        (window as unknown as Storing).glaveStored(
          thread,
          seq,
          new Date().toISOString(),
          type,
          data,
        );
      /** The steps as far as `done` of them are done, the next one doing. */
      const ticked = (done: number): readonly Step[] =>
        steps.map((step, index): Step => {
          if (index < done) {
            return { ...step, status: 'completed' };
          }
          return { ...step, status: index === done ? 'in_progress' : 'pending' };
        });
      window.fetch = async (input, init) => {
        const body = typeof init?.body === 'string' ? init.body : '';
        const message = body === '' ? null : (JSON.parse(body) as Record<string, unknown>);
        const params = (message?.['params'] ?? {}) as Record<string, unknown>;
        const id = Number(message?.['id']);
        if (message === null || params['thread'] !== thread) {
          return real(input, init);
        }
        if (message['method'] === 'thread/plan') {
          return answered(id, { plan: plan() });
        }
        if (message['method'] === 'plan/edit') {
          const response = await real(input, init);
          const kept = (await response.clone().json()) as { result?: { plan?: { steps: Step[] } } };
          steps = kept.result?.plan?.steps ?? steps;
          return response;
        }
        if (message['method'] !== 'plan/build') {
          return real(input, init);
        }
        // The server records that it runs in Auto from now on, which its lists then say.
        void real(input, init);
        const stream = new ReadableStream<Uint8Array>({
          async start(controller) {
            let seq = after;
            const send = (note: string): void => {
              controller.enqueue(encoder.encode(`data: ${note}\n\n`));
            };
            const tick = (call: string, done: number): void => {
              const now = ticked(done);
              send(
                stored((seq += 1), 'tool.call.requested', {
                  call_id: call,
                  tool: 'update_plan',
                  arguments: { json: { plan: now } },
                  summary: `update_plan ${String(done)}/${String(now.length)} done`,
                }),
              );
              steps = now;
              send(stored((seq += 1), 'plan.updated', { steps: now, call_id: call }));
              send(
                stored((seq += 1), 'tool.call.completed', {
                  call_id: call,
                  is_error: false,
                  output: [{ type: 'text', text: 'Plan updated.' }],
                }),
              );
            };
            proposing = false;
            send(stored((seq += 1), 'plan.approved', {}));
            send(stored((seq += 1), 'permissions.chosen', { permissions: 'auto' }));
            send(stored((seq += 1), 'turn.started', {}));
            const built = { type: 'text', text: 'The person approved this plan. Build it now.' };
            send(
              stored((seq += 1), 'message.added', {
                role: 'user',
                parts: [built],
                author: { kind: 'glave' },
              }),
            );
            tick('call_3', 1);
            await (window as unknown as Gated).glaveGates.passed('step');
            tick('call_4', steps.length);
            const reply = {
              type: 'text',
              text: 'Added `--verbose`: each step it takes is printed, and the tests pass.',
            };
            send(stored((seq += 1), 'message.added', { role: 'assistant', parts: [reply] }));
            send(stored((seq += 1), 'turn.ended', { outcome: 'completed' }));
            const tokens = { input: 1, output: 1, cache_read: 0, cache_write: 0, reasoning: 0 };
            const turn = { outcome: 'completed', model_calls: 3, tool_calls: 2, tokens };
            send(JSON.stringify({ jsonrpc: '2.0', id, result: { turn } }));
            controller.close();
          },
        });
        return new Response(stream, { headers: { 'Content-Type': 'text/event-stream' } });
      };
    },
    { thread: PLANNING, proposed: PROPOSED, after: PLANNING_HISTORY.length },
  );
}
