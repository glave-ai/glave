import { DOCUMENT } from '@angular/common';
import { InjectionToken, Service, computed, inject, signal } from '@angular/core';
import {
  ApiError,
  GlaveClient,
  type Method,
  type Methods,
  type Notification,
  type StreamingMethod,
} from '@glave/client';

import { forget, storageOf } from '../lib/storage';
import { TOKEN_KEY, takeToken } from '../lib/token';

/** The calls the app makes of the session API: `GlaveClient`'s, or a test's stand-in. */
export interface SessionClient {
  call<M extends Method>(
    method: M,
    params: Methods[M]['params'],
    signal?: AbortSignal,
  ): Promise<Methods[M]['result']>;
  stream<M extends StreamingMethod>(
    method: M,
    params: Methods[M]['params'],
    signal?: AbortSignal,
  ): AsyncGenerator<Notification, Methods[M]['result'], void>;
}

/** Makes the client for the server that served the app, signing each call in with `token`. */
export const SESSION_CLIENT_FACTORY = new InjectionToken<(token: () => string) => SessionClient>(
  'SESSION_CLIENT_FACTORY',
  {
    providedIn: 'root',
    factory: () => {
      const document = inject(DOCUMENT);
      const url = new URL('/', document.location.href).toString();
      return (token) => new GlaveClient({ url, token });
    },
  },
);

/**
 * The person's session with the glave that served the app: the sign-in token (from the
 * link's fragment, kept for this tab) and the client every call goes through. A call the
 * server refuses for want of a sign-in signs the app out.
 */
@Service()
export class SessionService {
  private readonly document = inject(DOCUMENT);
  private readonly storage = storageOf(this.document, 'sessionStorage');
  private readonly token = signal<string | null>(this.read());

  readonly signedIn = computed(() => this.token() !== null);
  private readonly client = inject(SESSION_CLIENT_FACTORY)(() => this.token() ?? '');

  /** Calls `method`; a refused sign-in signs the app out before the error is thrown on. */
  async call<M extends Method>(
    method: M,
    params: Methods[M]['params'],
    signal?: AbortSignal,
  ): Promise<Methods[M]['result']> {
    try {
      return await this.client.call(method, params, signal);
    } catch (error: unknown) {
      this.noticeSignOut(error);
      throw error;
    }
  }

  /** Starts a turn or goal, yielding its notifications; the generator returns its result. */
  async *stream<M extends StreamingMethod>(
    method: M,
    params: Methods[M]['params'],
    signal?: AbortSignal,
  ): AsyncGenerator<Notification, Methods[M]['result'], void> {
    try {
      return yield* this.client.stream(method, params, signal);
    } catch (error: unknown) {
      this.noticeSignOut(error);
      throw error;
    }
  }

  signOut(): void {
    this.token.set(null);
    forget(this.storage, TOKEN_KEY);
  }

  private noticeSignOut(error: unknown): void {
    if (error instanceof ApiError && error.signedOut) {
      this.signOut();
    }
  }

  private read(): string | null {
    const view = this.document.defaultView;
    if (view === null) {
      return null;
    }
    return takeToken({ location: view.location, history: view.history, storage: this.storage });
  }
}
