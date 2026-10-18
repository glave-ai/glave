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
import {
  type BrowserSession,
  TOKEN_KEY,
  isLocalToken,
  keepSession,
  readSession,
  sessionFor,
  takeCode,
} from '../lib/token';

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

/**
 * The session the tab starts with, before `start` signs it in: none in the app, a test's own
 * in tests.
 */
export const INITIAL_SESSION = new InjectionToken<BrowserSession | null>('INITIAL_SESSION', {
  providedIn: 'root',
  factory: () => null,
});

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
 * The person's session with the glave that served the app: a browser session (traded for the
 * link's one-time code, kept for this tab, and presented only to the server that made it) and
 * the client every call goes through. A call the server refuses for want of a sign-in signs
 * the app out.
 */
@Service()
export class SessionService {
  private readonly document = inject(DOCUMENT);
  private readonly storage = storageOf(this.document, 'sessionStorage');
  private readonly session = signal<BrowserSession | null>(inject(INITIAL_SESSION));

  readonly signedIn = computed(() => this.session() !== null);
  /** Whether the session is with a glave on this computer: a browser session. */
  readonly local = computed(() => {
    const session = this.session();
    return session !== null && isLocalToken(session.token);
  });
  private readonly client = inject(SESSION_CLIENT_FACTORY)(() => this.session()?.token ?? '');

  /**
   * Signs the tab in, before the app starts: with the link's code when it carries one, else
   * with the session the tab kept, and either only for the server that served the app, as
   * its `server/discover` names itself. A server it cannot reach, or another, leaves the tab
   * signed out, its session unsent.
   */
  async start(): Promise<void> {
    const view = this.document.defaultView;
    if (view === null) {
      return;
    }
    const code = takeCode({ location: view.location, history: view.history });
    let server: string | undefined;
    try {
      server = (await this.client.call('server/discover', {})).id ?? undefined;
    } catch {
      return;
    }
    if (code !== null && server !== undefined && (await this.redeem(code, server))) {
      return;
    }
    this.session.set(sessionFor(readSession(this.storage), server));
  }

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

  /** Signs this tab out: its session ends with the server, and the tab forgets it. */
  async signOut(): Promise<void> {
    if (this.session() !== null) {
      try {
        await this.client.call('signin/end', {});
      } catch {
        // Ended already, or the server is away: the tab forgets it all the same.
      }
    }
    this.forget();
  }

  /** Signs every other browser out; this tab stays signed in. */
  async signOutOthers(): Promise<void> {
    await this.call('signin/end_others', {});
  }

  /** Trades `code` for a session with `server`; whether it did. */
  private async redeem(code: string, server: string): Promise<boolean> {
    try {
      const made = await this.client.call('signin/redeem', { code });
      if (made.server !== server) {
        return false;
      }
      const session = { token: made.session, server };
      keepSession(this.storage, session);
      this.session.set(session);
      return true;
    } catch {
      // Used or expired: the session the tab kept, if any, is tried instead.
      return false;
    }
  }

  private forget(): void {
    this.session.set(null);
    forget(this.storage, TOKEN_KEY);
  }

  private noticeSignOut(error: unknown): void {
    if (error instanceof ApiError && error.signedOut) {
      this.forget();
    }
  }
}
