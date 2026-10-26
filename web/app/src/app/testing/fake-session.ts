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

/** What a turn the test drives sends next: a notification, or its result or error, last. */
type Sent =
  | { readonly kind: 'note'; readonly note: Notification }
  | { readonly kind: 'end'; readonly result: unknown }
  | { readonly kind: 'fail'; readonly error: unknown };

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

  /** Ends it with `error`, as a stream cut off or refused does. */
  fail(error: unknown): void {
    this.sent.push({ kind: 'fail', error });
    this.wake?.();
  }

  /** What the test sends, until it ends; stopped by `signal`, as a request is aborted. */
  async *run<R>(signal?: AbortSignal): AsyncGenerator<Notification, R, void> {
    for (;;) {
      const next = this.sent.shift();
      if (next === undefined) {
        await new Promise<void>((resolve, reject) => {
          this.wake = resolve;
          signal?.addEventListener(
            'abort',
            () => {
              reject(new DOMException('The request was aborted', 'AbortError'));
            },
            { once: true },
          );
        });
      } else if (next.kind === 'end') {
        return next.result as R;
      } else if (next.kind === 'fail') {
        throw next.error;
      } else {
        yield next.note;
      }
    }
  }
}

/**
 * A session API in memory for tests: answers by method, notifications for a turn and for a
 * watch, and every call recorded. An answer that is an `ApiError` is thrown. Unanswered,
 * `thread/get` answers the thread as the fixtures make one, idle and asking nothing, and
 * `thread/rename` the same thread with its new title, updated since;
 * `thread/sources`, no reply drew on the web; `thread/plan`, no plan;
 * `command/list`, nothing running; `awake/get`, a server, which keeps no computer awake;
 * `search/get`, web search not the person's to choose;
 * `grouping/get`, by project, and `grouping/set`, the grouping asked for; a watch, of a thread
 * or of the lists, sends nothing until it is stopped; any other method, `{}`.
 */
export class FakeSession implements SessionClient {
  readonly calls: SeenCall[] = [];
  private readonly answers = new Map<string, Answer>();
  private notes: readonly Notification[] = [];
  private turnResult: unknown = COMPLETED;
  /** How the next `turn/start`s are answered, in order: a turn the test drives, or a result. */
  private readonly turns: (FakeTurn | object)[] = [];
  /** What the next `thread/watch`es send, in order, each driven by the test. */
  private readonly watches: FakeTurn[] = [];
  /** What the next `thread/list/watch`es send, in order, each driven by the test. */
  private readonly listWatches: FakeTurn[] = [];

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

  /** Answers the next `thread/watch` with `watch`, which the test drives. */
  watch(watch: FakeTurn): this {
    this.watches.push(watch);
    return this;
  }

  /** Answers the next `thread/list/watch` with `watch`, which the test drives. */
  listWatch(watch: FakeTurn): this {
    this.listWatches.push(watch);
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
    signal?: AbortSignal,
  ): AsyncGenerator<Notification, Methods[M]['result'], void> {
    this.calls.push({ method, params });
    if (method === 'thread/watch' || method === 'thread/list/watch') {
      const waiting = method === 'thread/watch' ? this.watches : this.listWatches;
      const watch = waiting.shift() ?? new FakeTurn();
      return yield* watch.run<Methods[M]['result']>(signal);
    }
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

/** When a thread the fake renames was updated: after the fixtures' threads were. */
const RENAMED_AT = '2026-10-11T10:06:00.000Z';

/** What the fake answers a method no test answered. */
function unanswered(method: Method, params: unknown): Answer {
  if (method === 'thread/get') {
    const { thread: id } = params as { readonly thread: string };
    return { thread: thread(id, null) };
  }
  if (method === 'thread/rename') {
    const { thread: id, title } = params as { readonly thread: string; readonly title: string };
    return { thread: thread(id, title, { updated_at: RENAMED_AT }) };
  }
  if (method === 'thread/sources') {
    return { sources: [] };
  }
  if (method === 'thread/plan') {
    return { plan: null };
  }
  if (method === 'command/list') {
    return { commands: [] };
  }
  if (method === 'awake/get') {
    return { awake: { kind: 'never' } };
  }
  if (method === 'search/get') {
    return {
      search: {
        kind: 'never',
        reason: 'web search is set up by whoever runs this server, not from here',
      },
    };
  }
  if (method === 'grouping/get') {
    return { grouping: 'project' };
  }
  if (method === 'grouping/set') {
    const { grouping } = params as { readonly grouping: string };
    return { grouping };
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
