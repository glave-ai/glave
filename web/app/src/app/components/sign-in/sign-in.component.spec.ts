import { type ComponentFixture, TestBed } from '@angular/core/testing';
import type { MockInstance } from 'vitest';
import { Router, provideRouter } from '@angular/router';
import { expectAccessible } from '@glave/glaze/testing';

import { FAKE_SERVER, FakeSession, provideFakeSession } from '../../testing/fake-session';
import type { Authenticator } from '../../lib/passkeys';
import { AUTHENTICATOR, PLATFORM, passkeyRefused } from '../../services/passkeys.service';
import { SessionService } from '../../services/session.service';
import { SignInComponent } from './sign-in.component';

/** What a passkey's authenticator answers: one shaped as the browser's are. */
const ANSWER = {
  id: 'BAU',
  rawId: new Uint8Array([4, 5]).buffer,
  type: 'public-key',
  response: {
    attestationObject: new Uint8Array([1]).buffer,
    authenticatorData: new Uint8Array([1]).buffer,
    clientDataJSON: new Uint8Array([2]).buffer,
    signature: new Uint8Array([3]).buffer,
    userHandle: null,
    getTransports: () => ['internal'],
  },
  getClientExtensionResults: () => ({}),
};

const OPTIONS = {
  ceremony: 'c1',
  options: {
    publicKey: {
      challenge: 'AQID',
      rp: { id: 'glave.test', name: 'glave' },
      rpId: 'glave.test',
      user: { id: 'AQID', name: 'glave', displayName: 'glave' },
      pubKeyCredParams: [],
    },
  },
};

/** A glave answering `discovered` to `server/discover`, signing browsers in with passkeys. */
function signingIn(discovered: object): FakeSession {
  return new FakeSession()
    .answer('server/discover', { id: FAKE_SERVER, ...discovered })
    .answer('passkey/get_options', OPTIONS)
    .answer('passkey/create_options', OPTIONS)
    .answer('passkey/get', { session: 'glave_session_got', server: FAKE_SERVER })
    .answer('passkey/create', { session: 'glave_session_made', server: FAKE_SERVER });
}

/** A glave reached from elsewhere, signing browsers in with passkeys. */
function reached(): FakeSession {
  return signingIn({ origin: 'https://glave.test', signin: ['link', 'passkey'] });
}

/** The sign-in page, signed out, on `fake`, with `authenticator` making and using passkeys. */
async function render(
  fake: FakeSession,
  authenticator: Authenticator,
): Promise<{
  readonly fixture: ComponentFixture<SignInComponent>;
  readonly navigate: MockInstance<Router['navigateByUrl']>;
  readonly root: HTMLElement;
}> {
  TestBed.configureTestingModule({
    providers: [
      provideRouter([]),
      ...provideFakeSession(fake, null),
      { provide: AUTHENTICATOR, useValue: authenticator },
      { provide: PLATFORM, useValue: 'MacIntel' },
    ],
  });
  await TestBed.inject(SessionService).start();
  const navigate = vi.spyOn(TestBed.inject(Router), 'navigateByUrl').mockResolvedValue(true);
  const fixture = TestBed.createComponent(SignInComponent);
  await fixture.whenStable();
  return { fixture, navigate, root: fixture.nativeElement as HTMLElement };
}

const answering = (answer: unknown): Authenticator => ({
  create: () => Promise.resolve(answer as Credential),
  get: () => Promise.resolve(answer as Credential),
});

describe('SignInComponent', () => {
  afterEach(() => {
    window.history.replaceState(null, '', '/');
    window.localStorage.clear();
  });

  it('says how to sign in, in plain words', async () => {
    TestBed.configureTestingModule({
      providers: [provideRouter([]), ...provideFakeSession(new FakeSession(), null)],
    });
    const fixture = TestBed.createComponent(SignInComponent);
    await fixture.whenStable();
    const root: HTMLElement = fixture.nativeElement;
    expect(root.querySelector('header glz-top-bar')?.textContent).toContain('glave');
    const said = root.querySelector('main')?.textContent ?? '';
    expect(said).toContain('Open glave on this computer to sign this browser in');
    expect(said).not.toMatch(/serve|--|config|terminal/u);
    expect(root.querySelector('[data-testid="sign-in-passkey"]')).toBeNull();
    await expectAccessible(root);
  });

  it('goes to the workspace once signed in', async () => {
    TestBed.configureTestingModule({
      providers: [provideRouter([]), ...provideFakeSession(new FakeSession())],
    });
    const navigate = vi.spyOn(TestBed.inject(Router), 'navigateByUrl').mockResolvedValue(true);
    const fixture = TestBed.createComponent(SignInComponent);
    await fixture.whenStable();
    expect(navigate).toHaveBeenCalledWith('/');
  });

  it('signs in with a passkey where glave is reached from elsewhere', async () => {
    const fake = reached();
    const { fixture, navigate, root } = await render(fake, answering(ANSWER));
    const said = root.querySelector('main')?.textContent ?? '';
    expect(said).toContain('Sign in to glave');
    expect(said).not.toMatch(/serve|--|config|terminal|passkey\//u);
    await expectAccessible(root);
    root.querySelector<HTMLButtonElement>('[data-testid="sign-in-passkey"]')?.click();
    await fixture.whenStable();
    expect(fake.paramsOf('passkey/get')).toEqual([
      expect.objectContaining({ ceremony: 'c1' }) as unknown,
    ]);
    expect(navigate).toHaveBeenCalledWith('/');
  });

  it('makes a passkey with the setup link it was opened with', async () => {
    window.history.replaceState(null, '', '/#setup=f00d');
    const fake = reached();
    const { fixture, navigate, root } = await render(fake, answering(ANSWER));
    expect(window.location.hash).toBe('');
    expect(root.querySelector('main')?.textContent).toContain('Make a passkey for this device');
    root.querySelector<HTMLButtonElement>('[data-testid="sign-in-passkey"]')?.click();
    await fixture.whenStable();
    expect(fake.paramsOf('passkey/create_options')).toEqual([{ code: 'f00d' }]);
    expect(navigate).toHaveBeenCalledWith('/');
  });

  it('says why when the device makes or uses no passkey', async () => {
    const { fixture, navigate, root } = await render(reached(), answering(null));
    root.querySelector<HTMLButtonElement>('[data-testid="sign-in-passkey"]')?.click();
    await fixture.whenStable();
    const problem = root.querySelector('[data-testid="sign-in-problem"]');
    expect(problem?.textContent).toContain(passkeyRefused());
    expect(problem?.getAttribute('role')).toBe('alert');
    expect(navigate).not.toHaveBeenCalled();
    await expectAccessible(root);
  });

  it('on this computer, signs in with Touch ID once a passkey was made here', async () => {
    const fake = signingIn({ here: window.location.origin, signin: ['link', 'passkey'] });
    const { fixture, navigate, root } = await render(fake, answering(ANSWER));
    const said = root.querySelector('main')?.textContent ?? '';
    expect(said).toContain('the passkey you made for glave on this computer');
    expect(said).toContain('Open glave on this computer: it signs this browser in');
    const button = root.querySelector<HTMLButtonElement>('[data-testid="sign-in-passkey"]');
    expect(button?.textContent.trim()).toBe('Sign in with Touch ID');
    await expectAccessible(root);
    button?.click();
    await fixture.whenStable();
    expect(fake.paramsOf('passkey/get')).toHaveLength(1);
    expect(navigate).toHaveBeenCalledWith('/');
  });

  it('on this computer, offers no passkey before one is made, nor at another address', async () => {
    for (const discovered of [
      { here: window.location.origin, signin: ['link'] },
      { here: 'http://localhost:1', signin: ['link', 'passkey'] },
    ]) {
      TestBed.resetTestingModule();
      const { root } = await render(signingIn(discovered), answering(ANSWER));
      expect(root.querySelector('[data-testid="sign-in-passkey"]')).toBeNull();
      expect(root.querySelector('main')?.textContent).toContain(
        'Open glave on this computer to sign this browser in',
      );
    }
  });
});
