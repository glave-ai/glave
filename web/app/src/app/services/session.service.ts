import { DOCUMENT } from '@angular/common';
import { InjectionToken, Service, computed, inject, signal } from '@angular/core';
import {
  ApiError,
  GlaveClient,
  type Method,
  type Methods,
  type Notification,
  type SignInWay,
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
  /** How the server signs browsers in, and its id, as it said when the app started. */
  private readonly ways = signal<readonly SignInWay[]>([]);
  private server: string | undefined;
  private readonly setupLink = signal<string | null>(null);

  readonly signedIn = computed(() => this.session() !== null);
  /** Whether a browser signs in to the server with a passkey: it is reached from elsewhere. */
  readonly passkeys = computed(() => this.ways().includes('passkey'));
  /** The code of the setup link the tab was opened with, until the tab signs in. */
  readonly setupCode = this.setupLink.asReadonly();
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
    const place = { location: view.location, history: view.history };
    const code = takeCode(place, 'code');
    this.setupLink.set(takeCode(place, 'setup'));
    try {
      const discovered = await this.client.call('server/discover', {});
      this.server = discovered.id ?? undefined;
      this.ways.set(discovered.signin ?? []);
    } catch {
      return;
    }
    if (code !== null && (await this.redeem(code))) {
      return;
    }
    this.session.set(sessionFor(readSession(this.storage), this.server));
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

  /**
   * Keeps for this tab the session a server began, as a link's code or a passkey began it:
   * whether the server that served the app began it, the only one it is presented to.
   */
  begin(began: { readonly session: string; readonly server: string }): boolean {
    if (this.server === undefined || began.server !== this.server) {
      return false;
    }
    const session = { token: began.session, server: began.server };
    keepSession(this.storage, session);
    this.session.set(session);
    this.setupLink.set(null);
    return true;
  }

  /** Trades `code` for a session with the server that served the app; whether it did. */
  private async redeem(code: string): Promise<boolean> {
    try {
      return this.begin(await this.client.call('signin/redeem', { code }));
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
