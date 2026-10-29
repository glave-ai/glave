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

import { type PasskeySignals, signalAccepted } from '../lib/passkey-signals';
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

/** The browser's `PublicKeyCredential`, for its Signal API, where it has one. */
export const PASSKEY_SIGNALS = new InjectionToken<PasskeySignals | null>('PASSKEY_SIGNALS', {
  providedIn: 'root',
  factory: () => inject(DOCUMENT).defaultView?.PublicKeyCredential ?? null,
});

/** A session a server began, by a link's code or a passkey: each answers it alike. */
export type Began = Methods['signin/redeem']['result'];

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
 * link's one-time code, or begun with a passkey, kept in this origin's storage for every tab
 * of this browser, and presented only to the server that made it) and the client every call
 * goes through. A call the server refuses for want of a sign-in signs the app out, and a tab
 * that signs in or out takes every other tab of this browser with it.
 */
@Service()
export class SessionService {
  private readonly document = inject(DOCUMENT);
  private readonly storage = storageOf(this.document);
  private readonly session = signal<BrowserSession | null>(inject(INITIAL_SESSION));
  /** How the server signs browsers in, and its id, as it said when the app started. */
  private readonly ways = signal<readonly SignInWay[]>([]);
  /** Whether people reach the server from elsewhere, at its public origin. */
  private readonly reachedElsewhere = signal(false);
  /** Where browsers on this computer make passkeys for it, when it makes them here. */
  private readonly here = signal<string | null>(null);
  /** The version of glave whose app this page is, as the server said when the page loaded. */
  private readonly loaded = signal<string | null>(null);
  private server: string | undefined;
  private readonly setupLink = signal<string | null>(null);
  private readonly inviteLink = signal<string | null>(null);
  private readonly signupLink = signal<string | null>(null);

  readonly signedIn = computed(() => this.session() !== null);
  /**
   * The version of glave this page's code is of: the server's when the page loaded, or `null`
   * when it could not be reached then.
   */
  readonly loadedVersion = this.loaded.asReadonly();
  /** Whether glave makes passkeys for browsers on this computer, at this page's origin. */
  readonly passkeysHere = computed(() => this.here() === this.document.location.origin);
  /**
   * Whether this page makes passkeys and signs in with them: at the server's public origin, or
   * on this computer where glave makes them here.
   */
  readonly passkeys = computed(
    () => (this.reachedElsewhere() && this.ways().includes('passkey')) || this.passkeysHere(),
  );
  /** Whether a passkey signs this page in now: one exists, or a setup link makes the first. */
  readonly passkeySignIn = computed(() => this.passkeys() && this.ways().includes('passkey'));
  /** The code of the setup link the tab was opened with, until the tab signs in. */
  readonly setupCode = this.setupLink.asReadonly();
  /** The code of the invite the tab was opened with, until the person joins or puts it aside. */
  readonly inviteCode = this.inviteLink.asReadonly();
  /** The code of the sign-up link the tab was opened with, until the tab signs in. */
  readonly signupCode = this.signupLink.asReadonly();
  /** Whether anyone may sign up here with their email, as the server said. */
  readonly signsUp = computed(() => this.reachedElsewhere() && this.ways().includes('signup'));
  /** Whether the session is with a glave on this computer: a browser session. */
  readonly local = computed(() => {
    const session = this.session();
    return session !== null && isLocalToken(session.token);
  });
  /**
   * Whether to offer making a passkey here: a browser on this computer signed in by glave's
   * link, where glave makes passkeys at this page and none exists yet.
   */
  readonly offersPasskey = computed(
    () => this.local() && this.passkeysHere() && !this.ways().includes('passkey'),
  );
  private readonly client = inject(SESSION_CLIENT_FACTORY)(() => this.session()?.token ?? '');
  private readonly passkeySignals = inject(PASSKEY_SIGNALS);

  /**
   * Signs the tab in, before the app starts: with the link's code when it carries one, else
   * with the session this browser kept, and either only for the server that served the app,
   * as its `server/discover` names itself. A server it cannot reach, or another, leaves the
   * tab signed out, its session unsent. From then on the tab follows the session another tab
   * keeps or forgets.
   */
  async start(): Promise<void> {
    const view = this.document.defaultView;
    if (view === null) {
      return;
    }
    const place = { location: view.location, history: view.history };
    const code = takeCode(place, 'code');
    this.setupLink.set(takeCode(place, 'setup'));
    this.inviteLink.set(takeCode(place, 'invite'));
    this.signupLink.set(takeCode(place, 'signup'));
    try {
      this.take(await this.client.call('server/discover', {}));
    } catch {
      return;
    }
    view.addEventListener('storage', (event) => {
      if (event.key === TOKEN_KEY || event.key === null) {
        this.session.set(sessionFor(readSession(this.storage), this.server));
      }
    });
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
    const presented = this.session()?.token;
    try {
      return await this.client.call(method, params, signal);
    } catch (error: unknown) {
      this.noticeSignOut(error, presented);
      throw error;
    }
  }

  /** Starts a turn or goal, yielding its notifications; the generator returns its result. */
  async *stream<M extends StreamingMethod>(
    method: M,
    params: Methods[M]['params'],
    signal?: AbortSignal,
  ): AsyncGenerator<Notification, Methods[M]['result'], void> {
    const presented = this.session()?.token;
    try {
      return yield* this.client.stream(method, params, signal);
    } catch (error: unknown) {
      this.noticeSignOut(error, presented);
      throw error;
    }
  }

  /** Signs this browser out: its session ends with the server, and every tab forgets it. */
  async signOut(): Promise<void> {
    // The session this browser keeps now, though this tab missed another's signing in.
    const kept = sessionFor(readSession(this.storage), this.server);
    if (kept !== null) {
      this.session.set(kept);
    }
    const presented = this.session()?.token;
    if (presented !== undefined) {
      try {
        await this.client.call('signin/end', {});
      } catch {
        // Ended already, or the server is away: the browser forgets it all the same.
      }
    }
    this.forget(presented);
  }

  /** Asks the server again how it signs browsers in, as once a passkey is made or removed. */
  async rediscover(): Promise<void> {
    try {
      this.take(await this.client.call('server/discover', {}));
    } catch {
      // Away: what it said before stands.
    }
  }

  /** Takes in what the server says it is and how it signs browsers in. */
  private take(discovered: Methods['server/discover']['result']): void {
    this.server = discovered.id ?? undefined;
    if (this.loaded() === null) {
      this.loaded.set(discovered.version);
    }
    this.ways.set(discovered.signin ?? []);
    this.reachedElsewhere.set(typeof discovered.origin === 'string');
    this.here.set(discovered.here ?? null);
  }

  /** Forgets the tab's sign-up link, once it opens nothing. */
  clearSignup(): void {
    this.signupLink.set(null);
  }

  /** Puts the tab's invite aside: the person joined, or chose not to now. */
  clearInvite(): void {
    this.inviteLink.set(null);
  }

  /** Signs every other browser out; this one stays signed in. */
  async signOutOthers(): Promise<void> {
    await this.call('signin/end_others', {});
  }

  /**
   * Keeps for this browser the session a server began, as a link's code or a passkey began
   * it: whether the server that served the app began it, the only one it is presented to. The
   * browser's passkey manager is then told which of the person's passkeys sign them in, where
   * the server named them, so it hides or deletes the rest.
   */
  begin(began: Began): boolean {
    if (this.server === undefined || began.server !== this.server) {
      return false;
    }
    const session = { token: began.session, server: began.server };
    keepSession(this.storage, session);
    this.session.set(session);
    this.setupLink.set(null);
    this.signupLink.set(null);
    const accepted = began.accepted ?? null;
    if (accepted !== null) {
      void signalAccepted(this.passkeySignals, accepted);
    }
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

  /**
   * Forgets the session `ended`: this tab's, and this browser's while it still keeps that one.
   * A newer one another tab began meanwhile stays, and this tab takes it.
   */
  private forget(ended: string | undefined): void {
    if (readSession(this.storage)?.token === ended) {
      forget(this.storage, TOKEN_KEY);
    }
    this.session.set(sessionFor(readSession(this.storage), this.server));
  }

  /** Forgets the session `presented` when `error` says it signs no one in. */
  private noticeSignOut(error: unknown, presented: string | undefined): void {
    if (error instanceof ApiError && error.signedOut) {
      this.forget(presented);
    }
  }
}
