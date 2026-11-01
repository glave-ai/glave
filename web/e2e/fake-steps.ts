import type { CallRun, RawEvent, ThreadSteps } from '@glave/client';

/** Whether `event` says something: a message with words, the person's or the model's. */
function says(event: RawEvent): boolean {
  const parts = event.data['parts'];
  return (
    event.type === 'message.added' &&
    Array.isArray(parts) &&
    parts.some(
      (part: { readonly type?: unknown; readonly text?: unknown }) =>
        part.type === 'text' && typeof part.text === 'string' && part.text.trim() !== '',
    )
  );
}

/** A run's line, as the fake server tells it. */
function sentence(calls: number): string {
  return `Ran ${String(calls)} ${calls === 1 ? 'call' : 'calls'}`;
}

/**
 * What the fake server says of `events`' steps, as the server reads a thread's log: each turn's
 * calls with nothing said between them as a run, questions apart, and each ended turn's last
 * words of the model's as its reply.
 */
export function stepsOf(events: readonly RawEvent[]): ThreadSteps {
  const runs: { first: number; calls: string[] }[] = [];
  const replies: number[] = [];
  let open = false;
  let words: number | null = null;
  for (const event of events) {
    if (event.type === 'turn.started') {
      [open, words] = [false, null];
    } else if (event.type === 'turn.ended') {
      open = false;
      replies.push(...(words === null ? [] : [words]));
    } else if (
      event.type === 'service.asked' ||
      (event.type === 'tool.call.requested' && event.data['tool'] === 'question')
    ) {
      open = false;
    } else if (event.type === 'tool.call.requested') {
      const id = String(event.data['call_id']);
      const last = runs.at(-1);
      if (open && last !== undefined) {
        last.calls.push(id);
      } else {
        runs.push({ first: event.seq, calls: [id] });
      }
      open = true;
    } else if (says(event)) {
      open = false;
      words = event.data['role'] === 'assistant' ? event.seq : words;
    }
  }
  const told = runs.map((run): CallRun => ({ ...run, sentence: sentence(run.calls.length) }));
  return { runs: told, replies };
}
