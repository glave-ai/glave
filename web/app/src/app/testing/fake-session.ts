import type { Provider } from '@angular/core';
import {
  ApiError,
  type Method,
  type Methods,
  type Notification,
  type StreamingMethod,
} from '@glave/client';

import type { BrowserSession } from '../lib/token';
import { thread } from './fixtures';
import {
  INITIAL_SESSION,
  SESSION_CLIENT_FACTORY,
  type SessionClient,
} from '../services/session.service';

/** The server the fake is, by the id its `server/discover` answers. */
export const FAKE_SERVER = 'fake-server';

/** A tab signed in to the fake: with a browser session, so on this computer. */
export const SIGNED_IN: BrowserSession = { token: 'glave_session_test', server: FAKE_SERVER };

/** A tab signed in to the fake with a token from elsewhere, not a session of glave's own. */
export const ELSEWHERE: BrowserSession = { token: 'an-access-token', server: FAKE_SERVER };

/** A call the fake saw. */
interface SeenCall {
  readonly method: string;
  readonly params: unknown;
}

type Answer = object | ((params: unknown) => unknown);

/** What a turn the test drives sends next: a notification, or its result, last. */
type Sent =
  | { readonly kind: 'note'; readonly note: Notification }
  | { readonly kind: 'end'; readonly result: unknown };

/** How a turn that completed ends, unless a test says otherwise. */
const COMPLETED = {
  turn: {
    outcome: 'completed',
    model_calls: 1,
    tool_calls: 0,
    tokens: { input: 0, output: 0, cache_read: 0, cache_write: 0, reasoning: 0 },
  },
};

/**
 * A turn a test drives while the app follows it: it streams what the test sends, when the test
 * sends it, then ends with the result the test gives.
 */
export class FakeTurn {
  private readonly sent: Sent[] = [];
  private wake: (() => void) | null = null;

  send(...notes: readonly Notification[]): void {
    this.sent.push(...notes.map((note): Sent => ({ kind: 'note', note })));
    this.wake?.();
  }

  end(result: unknown = COMPLETED): void {
    this.sent.push({ kind: 'end', result });
    this.wake?.();
  }

  async *run<R>(): AsyncGenerator<Notification, R, void> {
    for (;;) {
      const next = this.sent.shift();
      if (next === undefined) {
        await new Promise<void>((resolve) => {
          this.wake = resolve;
        });
      } else if (next.kind === 'end') {
        return next.result as R;
      } else {
        yield next.note;
      }
    }
  }
}

/**
 * A session API in memory for tests: answers by method, notifications for a turn, and every
 * call recorded. An answer that is an `ApiError` is thrown. Unanswered, `thread/get` answers
 * the thread as the fixtures make one, idle and asking nothing; `command/list`, nothing
 * running; any other method, `{}`.
 */
export class FakeSession implements SessionClient {
  readonly calls: SeenCall[] = [];
  private readonly answers = new Map<string, Answer>();
  private notes: readonly Notification[] = [];
  private turnResult: unknown = COMPLETED;
  /** How the next `turn/start`s are answered, in order: a turn the test drives, or a result. */
  private readonly turns: (FakeTurn | object)[] = [];

  answer(method: Method, answer: Answer): this {
    this.answers.set(method, answer);
    return this;
  }

  streams(notes: readonly Notification[], result?: unknown): this {
    this.notes = notes;
    if (result !== undefined) {
      this.turnResult = result;
    }
    return this;
  }

  /**
   * Answers the next `turn/start` with `turn`: a turn the test drives, a result answered at
   * once, such as a message queued to a turn already running, or an `ApiError`, thrown. After
   * these, every turn streams what `streams` set.
   */
  turn(turn: FakeTurn | object): this {
    this.turns.push(turn);
    return this;
  }

  call<M extends Method>(method: M, params: Methods[M]['params']): Promise<Methods[M]['result']> {
    this.calls.push({ method, params });
    const answer = this.answers.get(method) ?? unanswered(method, params);
    const value: unknown = typeof answer === 'function' ? answer(params) : answer;
    if (value instanceof ApiError) {
      return Promise.reject(value);
    }
    return Promise.resolve(value as Methods[M]['result']);
  }

  async *stream<M extends StreamingMethod>(
    method: M,
    params: Methods[M]['params'],
  ): AsyncGenerator<Notification, Methods[M]['result'], void> {
    this.calls.push({ method, params });
    const next = this.turns.shift();
    if (next instanceof FakeTurn) {
      return yield* next.run<Methods[M]['result']>();
    }
    if (next instanceof ApiError) {
      throw next;
    }
    if (next !== undefined) {
      return next as Methods[M]['result'];
    }
    for (const note of this.notes) {
      await Promise.resolve();
      yield note;
    }
    return this.turnResult as Methods[M]['result'];
  }

  /** The params of the calls of `method`, in order. */
  paramsOf(method: Method): readonly unknown[] {
    return this.calls.filter((call) => call.method === method).map((call) => call.params);
  }
}

/** What the fake answers a method no test answered. */
function unanswered(method: Method, params: unknown): Answer {
  if (method === 'thread/get') {
    const { thread: id } = params as { readonly thread: string };
    return { thread: thread(id, null) };
  }
  if (method === 'command/list') {
    return { commands: [] };
  }
  return {};
}

/** Providers that sign the app in with `session` (none: signed out) and send its calls to `fake`. */
export function provideFakeSession(
  fake: FakeSession,
  session: BrowserSession | null = SIGNED_IN,
): Provider[] {
  return [
    { provide: SESSION_CLIENT_FACTORY, useValue: () => fake },
    { provide: INITIAL_SESSION, useValue: session },
  ];
}
