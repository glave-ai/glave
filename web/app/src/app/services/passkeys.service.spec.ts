import { TestBed } from '@angular/core/testing';
import { ApiError, ErrorCode } from '@glave/client';
import type { Mock } from 'vitest';

import type { PasskeySignals } from '../lib/signals';
import { unreached } from '../lib/text';
import { FAKE_SERVER, FakeSession, provideFakeSession } from '../testing/fake-session';
import { AUTHENTICATOR, passkeyRefused, PasskeysService } from './passkeys.service';
import { PASSKEY_SIGNALS, SessionService } from './session.service';

const PHONE = { id: 'p1', name: 'Phone', added: '2026-10-20T09:00:00.000Z' };

/** What a passkey's authenticator answers to sign in: one shaped as the browser's are. */
const ASSERTION = {
  id: 'BAU',
  rawId: new Uint8Array([4, 5]).buffer,
  type: 'public-key',
  response: {
    authenticatorData: new Uint8Array([1]).buffer,
    clientDataJSON: new Uint8Array([2]).buffer,
    signature: new Uint8Array([3]).buffer,
    userHandle: new Uint8Array([1, 2, 3]).buffer,
  },
  getClientExtensionResults: () => ({}),
};

const ACCEPTED = { party: 'glave.test', user: 'AQID', credentials: ['BAU'] };

/** A glave reached from elsewhere, signing browsers in with passkeys. */
function reached(): FakeSession {
  return new FakeSession()
    .answer('server/discover', { id: FAKE_SERVER, origin: 'https://glave.test' })
    .answer('passkey/get_options', {
      ceremony: 'c1',
      options: { publicKey: { challenge: 'AQID', rpId: 'glave.test' } },
    })
    .answer('passkey/get', {
      session: 'glave_session_got',
      server: FAKE_SERVER,
      accepted: ACCEPTED,
    });
}

/** The passkeys of a tab signed out of `fake`, the browser's Signal API being `signals`. */
async function signedOut(
  fake: FakeSession,
  signals: PasskeySignals | null,
): Promise<PasskeysService> {
  TestBed.configureTestingModule({
    providers: [
      ...provideFakeSession(fake, null),
      {
        provide: AUTHENTICATOR,
        useValue: { create: () => Promise.resolve(null), get: () => Promise.resolve(ASSERTION) },
      },
      { provide: PASSKEY_SIGNALS, useValue: signals },
    ],
  });
  await TestBed.inject(SessionService).start();
  return TestBed.inject(PasskeysService);
}

/** A stand-in for the browser's `PublicKeyCredential` with the Signal API. */
function withSignals(): {
  readonly signalAllAcceptedCredentials: Mock<() => Promise<void>>;
  readonly signalUnknownCredential: Mock<() => Promise<void>>;
} {
  return {
    signalAllAcceptedCredentials: vi.fn(() => Promise.resolve()),
    signalUnknownCredential: vi.fn(() => Promise.resolve()),
  };
}

function service(fake: FakeSession, made: unknown = null): PasskeysService {
  TestBed.configureTestingModule({
    providers: [
      ...provideFakeSession(fake),
      {
        provide: AUTHENTICATOR,
        useValue: {
          create: () => Promise.resolve(made),
          get: () => Promise.resolve(null),
        },
      },
    ],
  });
  return TestBed.inject(PasskeysService);
}

describe('PasskeysService', () => {
  it('lists the passkeys, and removes one, listing them again', async () => {
    let kept = [PHONE];
    const fake = new FakeSession()
      .answer('passkey/list', () => ({ passkeys: kept }))
      .answer('passkey/remove', () => {
        kept = [];
        return {};
      });
    const keys = service(fake);
    expect(await keys.load()).toBeNull();
    expect(keys.passkeys()).toEqual([PHONE]);
    expect(await keys.remove('p1')).toBeNull();
    expect(keys.passkeys()).toEqual([]);
  });

  it('says why a passkey was not added, and why nothing could be listed', async () => {
    const fake = new FakeSession().answer('passkey/add_options', {
      ceremony: 'c1',
      options: { publicKey: { challenge: 'AQID', user: { id: 'AQID' } } },
    });
    const keys = service(fake);
    expect(await keys.add()).toBe(passkeyRefused());
    expect(fake.paramsOf('passkey/add')).toEqual([]);
    fake.answer('passkey/list', new ApiError({ code: ErrorCode.NOT_ALLOWED, message: 'no' }));
    expect(await keys.load()).toBe('no');
    fake.answer('passkey/list', () => {
      throw new TypeError('offline');
    });
    expect(await keys.load()).toBe(unreached());
  });

  it('signs in with a passkey, and tells the passkey manager every one that signs in', async () => {
    const signals = withSignals();
    const keys = await signedOut(reached(), signals);
    expect(await keys.signIn()).toBeNull();
    expect(TestBed.inject(SessionService).signedIn()).toBe(true);
    expect(signals.signalAllAcceptedCredentials).toHaveBeenCalledExactlyOnceWith({
      rpId: 'glave.test',
      userId: 'AQID',
      allAcceptedCredentialIds: ['BAU'],
    });
    expect(signals.signalUnknownCredential).not.toHaveBeenCalled();
  });

  it('tells the passkey manager of a passkey the server does not know, and says why', async () => {
    const signals = withSignals();
    const unknown = { unknown_passkey: { party: 'glave.test', credential: 'BAU' } };
    const fake = reached().answer(
      'passkey/get',
      new ApiError({ code: ErrorCode.NO_SIGN_IN, message: 'gone', data: unknown }),
    );
    const keys = await signedOut(fake, signals);
    expect(await keys.signIn()).toBe('gone');
    expect(signals.signalUnknownCredential).toHaveBeenCalledExactlyOnceWith({
      rpId: 'glave.test',
      credentialId: 'BAU',
    });
    expect(signals.signalAllAcceptedCredentials).not.toHaveBeenCalled();
    // A refusal that names no passkey tells the passkey manager nothing.
    fake.answer('passkey/get', new ApiError({ code: ErrorCode.NO_SIGN_IN, message: 'no' }));
    expect(await keys.signIn()).toBe('no');
    expect(signals.signalUnknownCredential).toHaveBeenCalledOnce();
  });

  it('signs in, and says why it refused, as before where the browser has no Signal API', async () => {
    for (const signals of [null, {}]) {
      TestBed.resetTestingModule();
      expect(await (await signedOut(reached(), signals)).signIn()).toBeNull();
      TestBed.resetTestingModule();
      const unknown = { unknown_passkey: { party: 'glave.test', credential: 'BAU' } };
      const fake = reached().answer(
        'passkey/get',
        new ApiError({ code: ErrorCode.NO_SIGN_IN, message: 'gone', data: unknown }),
      );
      expect(await (await signedOut(fake, signals)).signIn()).toBe('gone');
    }
  });
});
