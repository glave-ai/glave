import { type ComponentFixture, TestBed } from '@angular/core/testing';
import type { MockInstance } from 'vitest';
import { Router, provideRouter } from '@angular/router';
import { ApiError, ErrorCode } from '@glave/client';
import { expectAccessible } from '@glave/glaze/testing';

import type { Authenticator } from '../../lib/passkeys';
import { AUTHENTICATOR } from '../../services/passkeys.service';
import { SessionService } from '../../services/session.service';
import { FAKE_SERVER, FakeSession, provideFakeSession } from '../../testing/fake-session';
import { SignUpComponent } from './sign-up.component';

const OPTIONS = {
  ceremony: 'c1',
  options: {
    publicKey: {
      challenge: 'AQID',
      rp: { id: 'glave.test', name: 'glave' },
      user: { id: 'AQID', name: 'glave', displayName: 'glave' },
      pubKeyCredParams: [],
    },
  },
};

/** What a device answers when it makes a passkey, shaped as the browser's answers are. */
const MADE = {
  id: 'BAU',
  rawId: new Uint8Array([4, 5]).buffer,
  type: 'public-key',
  response: {
    attestationObject: new Uint8Array([1]).buffer,
    clientDataJSON: new Uint8Array([2]).buffer,
    getTransports: () => ['internal'],
  },
  getClientExtensionResults: () => ({}),
};

const device: Authenticator = {
  create: () => Promise.resolve(MADE as unknown as Credential),
  get: () => Promise.resolve(null),
};

async function settle(fixture: { whenStable(): Promise<unknown> }): Promise<void> {
  for (let turn = 0; turn < 5; turn += 1) {
    await new Promise((resolve) => setTimeout(resolve));
  }
  await fixture.whenStable();
}

/** The page opened at `link`, signed out, on a glave that takes sign-ups. */
async function render(
  fake: FakeSession,
  link = '/sign-up',
): Promise<{
  readonly fixture: ComponentFixture<SignUpComponent>;
  readonly navigate: MockInstance<Router['navigateByUrl']>;
  readonly root: HTMLElement;
}> {
  window.history.replaceState(null, '', link);
  const discovered = {
    id: FAKE_SERVER,
    origin: 'https://glave.test',
    signin: ['link', 'passkey', 'signup'],
  };
  TestBed.configureTestingModule({
    providers: [
      provideRouter([]),
      ...provideFakeSession(fake.answer('server/discover', discovered), null),
      { provide: AUTHENTICATOR, useValue: device },
    ],
  });
  await TestBed.inject(SessionService).start();
  const navigate = vi.spyOn(TestBed.inject(Router), 'navigateByUrl').mockResolvedValue(true);
  const fixture = TestBed.createComponent(SignUpComponent);
  await settle(fixture);
  return { fixture, navigate, root: fixture.nativeElement as HTMLElement };
}

function type(root: ParentNode, testId: string, text: string): void {
  const field = root.querySelector<HTMLInputElement>(`[data-testid="${testId}"]`);
  if (field !== null) {
    field.value = text;
    field.dispatchEvent(new Event('input'));
  }
}

describe('SignUpComponent', () => {
  afterEach(() => {
    localStorage.clear();
  });

  it('sends a link to the email given, then says to check it, and sends it again', async () => {
    const fake = new FakeSession().answer('signup/start', {});
    const { fixture, root } = await render(fake);
    expect(root.textContent).toContain('Get started with glave');
    expect(root.textContent).not.toMatch(/organization|team/iu);
    await expectAccessible(root);
    type(root, 'sign-up-address', ' ada@example.com ');
    root.querySelector<HTMLElement>('[data-testid="sign-up-send"]')?.click();
    await settle(fixture);
    expect(fake.paramsOf('signup/start')).toEqual([{ email: 'ada@example.com' }]);
    const sent = root.querySelector('[data-testid="sign-up-sent"]');
    expect(sent?.textContent).toContain('We sent a link to ada@example.com');
    await expectAccessible(root);
    root.querySelector<HTMLElement>('[data-testid="sign-up-again"]')?.click();
    await settle(fixture);
    expect(fake.paramsOf('signup/start')).toHaveLength(2);
  });

  it('opened from the emailed link, makes the passkey that makes the account', async () => {
    const fake = new FakeSession()
      .answer('passkey/create_options', OPTIONS)
      .answer('passkey/create', { session: 'glave_account_made', server: FAKE_SERVER });
    const { fixture, navigate, root } = await render(fake, '/#signup=c0de');
    expect(window.location.hash).toBe('');
    expect(root.querySelector('[data-testid="sign-up-passkey"]')?.textContent).toContain(
      'Make your passkey',
    );
    await expectAccessible(root);
    root.querySelector<HTMLElement>('[data-testid="sign-up-make"]')?.click();
    await settle(fixture);
    expect(fake.paramsOf('passkey/create_options')).toEqual([{ code: 'c0de', link: 'signup' }]);
    expect(TestBed.inject(SessionService).signedIn()).toBe(true);
    expect(navigate).toHaveBeenCalledWith('/');
  });

  it('says the server’s sentence when it refuses an address', async () => {
    const refused = new ApiError({
      code: ErrorCode.INVALID_PARAMS,
      message: "that isn't an email address: check it and try again",
    });
    const { fixture, root } = await render(new FakeSession().answer('signup/start', refused));
    type(root, 'sign-up-address', 'ada@example');
    root.querySelector<HTMLElement>('[data-testid="sign-up-send"]')?.click();
    await settle(fixture);
    expect(root.querySelector('[data-testid="sign-up-problem"]')?.textContent).toContain(
      "that isn't an email address",
    );
  });
});
