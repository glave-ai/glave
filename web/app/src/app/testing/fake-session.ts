import type { Provider } from '@angular/core';
import {
  ApiError,
  type Method,
  type Methods,
  type Notification,
  type StreamingMethod,
} from '@glave/client';

import { TOKEN_KEY } from '../lib/token';
import { thread } from './fixtures';
import { SESSION_CLIENT_FACTORY, type SessionClient } from '../services/session.service';

/** A call the fake saw. */
interface SeenCall {
  readonly method: string;
  readonly params: unknown;
}

type Answer = object | ((params: unknown) => unknown);

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
  private turnResult: unknown = {
    turn: {
      outcome: 'completed',
      model_calls: 1,
      tool_calls: 0,
      tokens: { input: 0, output: 0, cache_read: 0, cache_write: 0, reasoning: 0 },
    },
  };

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

/** Providers that sign the app in and send its calls to `fake`. */
export function provideFakeSession(fake: FakeSession, signedIn = true): Provider[] {
  if (signedIn) {
    sessionStorage.setItem(TOKEN_KEY, 'glave_local_test');
  } else {
    sessionStorage.removeItem(TOKEN_KEY);
  }
  return [{ provide: SESSION_CLIENT_FACTORY, useValue: () => fake }];
}
