import { DOCUMENT } from '@angular/common';
import { InjectionToken, Service, inject, signal } from '@angular/core';
import { ApiError, ErrorCode, type LinkKind, type PasskeySummary } from '@glave/client';

import { type Authenticator, made, used } from '../lib/passkeys';
import { type PlatformAuthenticator, platformAuthenticator } from '../lib/platform';
import { onChange } from './changes.service';
import { signalUnknown, unknownPasskey } from '../lib/passkey-signals';
import { unreached } from '../lib/text';
import { type Began, PASSKEY_SIGNALS, SessionService } from './session.service';

/** The browser's authenticator, where it has one. */
export const AUTHENTICATOR = new InjectionToken<Authenticator | null>('AUTHENTICATOR', {
  providedIn: 'root',
  factory: () => inject(DOCUMENT).defaultView?.navigator.credentials ?? null,
});

/** The system the browser runs on, as it names it: its client hints', else `navigator.platform`. */
export const PLATFORM = new InjectionToken<string>('PLATFORM', {
  providedIn: 'root',
  factory: () => {
    const navigator = inject(DOCUMENT).defaultView?.navigator;
    const hinted = (navigator as { userAgentData?: { platform?: string } } | undefined)
      ?.userAgentData?.platform;
    return hinted ?? navigator?.platform ?? '';
  },
});

/** Whether this computer has an authenticator of its own that checks who is there. */
export const PLATFORM_AUTHENTICATOR = new InjectionToken<() => Promise<boolean>>(
  'PLATFORM_AUTHENTICATOR',
  {
    providedIn: 'root',
    factory: () => {
      const credential = inject(DOCUMENT).defaultView?.PublicKeyCredential;
      return async () => {
        try {
          return (await credential?.isUserVerifyingPlatformAuthenticatorAvailable()) ?? false;
        } catch {
          return false;
        }
      };
    },
  },
);

/** The longest name the server takes, so a field stops there, before a device is asked. */
export const LONGEST_NAME = 100;

/** What the person is told when their device makes or uses no passkey: the app's own words. */
export function passkeyRefused(): string {
  return $localize`:@@passkeys.refused:Your device made or used no passkey. Try again, and allow it when your device asks.`;
}

/**
 * Passkeys, where glave is reached from elsewhere or makes them on this computer: making the
 * first with a setup link and signing in with one, which begin the browser's session; then
 * the person's passkeys, adding one on this device, and removing one, which signs out the
 * browsers it signed in. Each says `null` when it did, else why not, in a sentence: the
 * server's when it refused, after telling the browser's passkey manager of a passkey the
 * server named as one it does not know.
 */
@Service()
export class PasskeysService {
  private readonly session = inject(SessionService);
  private readonly authenticator = inject(AUTHENTICATOR);
  private readonly passkeySignals = inject(PASSKEY_SIGNALS);
  private readonly listed = signal<readonly PasskeySummary[]>([]);
  private readonly address = signal<string | null>(null);
  private readonly told = signal<string | null>(null);
  private readonly called = signal<string | null>(null);
  private readonly mayName = signal(false);

  /** The person's passkeys, oldest first, as the server last listed them. */
  readonly passkeys = this.listed.asReadonly();
  /** The address a glave account gets back in with, as the server last said. */
  readonly email = this.address.asReadonly();
  /** What the server tells of a passkey made by email lately, on another device. */
  readonly notice = this.told.asReadonly();
  /** The name the person goes by, as the server last said. */
  readonly personName = this.called.asReadonly();
  /** Whether they may change it, as the server last said: they have a glave account. */
  readonly nameable = this.mayName.asReadonly();
  /** This computer's own authenticator, as its system names it, where glave knows it. */
  readonly platform: PlatformAuthenticator = platformAuthenticator(inject(PLATFORM));

  constructor() {
    onChange('passkeys', () => void this.load());
  }

  /** Makes a passkey on this device with the tab's setup link, and signs the browser in. */
  async make(): Promise<string | null> {
    const code = this.session.setupCode();
    if (code === null) {
      return passkeyRefused();
    }
    return this.asking(async () => {
      const asked = await this.session.call('passkey/create_options', { code });
      const credential = await made(this.authenticator, asked.options);
      if (credential === null) {
        return passkeyRefused();
      }
      const created = { ceremony: asked.ceremony, credential };
      return this.begun(await this.session.call('passkey/create', created));
    });
  }

  /**
   * Makes the first passkey of a new account on this device with the invite `code`, for the
   * person who gives the name `personName`, joining the organization it is to, and signs the
   * browser in.
   */
  async join(code: string, personName: string): Promise<string | null> {
    return this.asking(() => this.makeWithLink(code, 'invite', personName));
  }

  /**
   * Makes a passkey on this device with the emailed link `code` of the kind `link`: a sign-up's,
   * the first of a new account, which it makes, for the person named `personName`; or one to
   * get back in, a new one for the account the address has. Either signs the browser in. Says
   * why not, if it did not, and whether the link opens nothing any more.
   */
  async withEmailedLink(
    code: string,
    link: 'signup' | 'recovery',
    personName: string | null,
  ): Promise<{ readonly problem: string | null; readonly spent: boolean }> {
    let spent = false;
    const problem = await this.asking(async () => {
      try {
        return await this.makeWithLink(code, link, personName);
      } catch (error: unknown) {
        spent = error instanceof ApiError && error.code === ErrorCode.NO_INVITE;
        throw error;
      }
    });
    return { problem, spent };
  }

  /** Signs the browser in with a passkey on this device. */
  async signIn(): Promise<string | null> {
    return this.asking(async () => {
      const asked = await this.session.call('passkey/get_options', {});
      const credential = await used(this.authenticator, asked.options);
      if (credential === null) {
        return passkeyRefused();
      }
      const got = { ceremony: asked.ceremony, credential };
      return this.begun(await this.session.call('passkey/get', got));
    });
  }

  /** Lists the person's passkeys again. */
  async load(): Promise<string | null> {
    return this.asking(async () => {
      const listed = await this.session.call('passkey/list', {});
      const { passkeys, email, notice } = listed;
      this.called.set(listed.person_name ?? null);
      this.mayName.set(listed.nameable ?? false);
      this.listed.set(passkeys);
      this.address.set(email ?? null);
      this.told.set(notice ?? null);
      return null;
    });
  }

  /** Changes the name the person goes by. */
  async rename(name: string): Promise<string | null> {
    return this.asking(async () => {
      await this.session.call('person/name', { name });
      return this.load();
    });
  }

  /** Adds a passkey on this device. */
  async add(): Promise<string | null> {
    return this.asking(async () => {
      const asked = await this.session.call('passkey/add_options', {});
      const credential = await made(this.authenticator, asked.options);
      if (credential === null) {
        return passkeyRefused();
      }
      await this.session.call('passkey/add', { ceremony: asked.ceremony, credential });
      await this.session.rediscover();
      return this.load();
    });
  }

  /** Removes the passkey `id`. */
  async remove(id: string): Promise<string | null> {
    return this.asking(async () => {
      await this.session.call('passkey/remove', { passkey: id });
      await this.session.rediscover();
      return this.load();
    });
  }

  /** Makes a new account's first passkey with a link's `code`, of the kind `link`. */
  private async makeWithLink(
    code: string,
    link: LinkKind,
    personName: string | null,
  ): Promise<string | null> {
    const asked = await this.session.call('passkey/create_options', { code, link });
    const credential = await made(this.authenticator, asked.options);
    if (credential === null) {
      return passkeyRefused();
    }
    const named = personName === null ? {} : { person_name: personName };
    const created = { ceremony: asked.ceremony, credential, ...named };
    return this.begun(await this.session.call('passkey/create', created));
  }

  /** Keeps the session a passkey began, when the server that served the app began it. */
  private begun(began: Began): string | null {
    return this.session.begin(began) ? null : passkeyRefused();
  }

  /** What `ask` says, or the server's sentence when it refused, or the app's own. */
  private async asking(ask: () => Promise<string | null>): Promise<string | null> {
    try {
      return await ask();
    } catch (error: unknown) {
      if (!(error instanceof ApiError)) {
        return unreached();
      }
      const unknown = unknownPasskey(error.data);
      if (unknown !== null) {
        void signalUnknown(this.passkeySignals, unknown);
      }
      return error.message;
    }
  }
}
