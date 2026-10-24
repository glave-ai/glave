import { TestBed } from '@angular/core/testing';
import { expectAccessible } from '@glave/glaze/testing';

import type { Authenticator } from '../../lib/passkeys';
import { TOKEN_KEY } from '../../lib/token';
import {
  AUTHENTICATOR,
  PLATFORM,
  PLATFORM_AUTHENTICATOR,
  passkeyRefused,
} from '../../services/passkeys.service';
import { SessionService } from '../../services/session.service';
import {
  FAKE_SERVER,
  FakeSession,
  SIGNED_IN,
  provideFakeSession,
} from '../../testing/fake-session';
import { PasskeyOfferComponent } from './passkey-offer.component';

/** What the authenticator answers when it makes a passkey, shaped as the browser's are. */
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

const OPTIONS = {
  ceremony: 'c1',
  options: {
    publicKey: {
      challenge: 'AQID',
      rp: { id: 'localhost', name: 'glave' },
      user: { id: 'AQID', name: 'glave', displayName: 'glave' },
      pubKeyCredParams: [],
    },
  },
};

interface Setting {
  /** What `server/discover` answers. */
  readonly discovered?: object;
  /** Whether this computer has an authenticator of its own. */
  readonly available?: boolean;
  /** What the authenticator answers: `null` when it makes nothing. */
  readonly answer?: unknown;
}

/** A glave that makes passkeys at this page's origin, none made yet. */
function here(): object {
  return { id: FAKE_SERVER, here: window.location.origin, signin: ['link'] };
}

/** The offer in a tab signed in by glave's link, on a Mac. */
async function render(setting: Setting = {}): Promise<{
  readonly fake: FakeSession;
  readonly root: HTMLElement;
  readonly settle: () => Promise<void>;
}> {
  let made = false;
  const discovered = setting.discovered ?? here();
  const fake = new FakeSession()
    .answer('server/discover', () =>
      made ? { ...discovered, signin: ['link', 'passkey'] } : discovered,
    )
    .answer('passkey/add_options', OPTIONS)
    .answer('passkey/add', () => {
      made = true;
      return { passkey: { id: 'p1', name: 'Mac', added: '2026-10-22T00:00:00Z' } };
    })
    .answer('passkey/list', { passkeys: [] });
  const authenticator: Authenticator = {
    create: () =>
      Promise.resolve((setting.answer === undefined ? MADE : setting.answer) as Credential),
    get: () => Promise.resolve(null),
  };
  TestBed.configureTestingModule({
    providers: [
      ...provideFakeSession(fake, SIGNED_IN),
      { provide: AUTHENTICATOR, useValue: authenticator },
      { provide: PLATFORM, useValue: 'MacIntel' },
      {
        provide: PLATFORM_AUTHENTICATOR,
        useValue: () => Promise.resolve(setting.available ?? true),
      },
    ],
  });
  // Signed in by glave's link: the session this browser keeps.
  localStorage.setItem(TOKEN_KEY, JSON.stringify(SIGNED_IN));
  await TestBed.inject(SessionService).start();
  const fixture = TestBed.createComponent(PasskeyOfferComponent);
  const settle = async (): Promise<void> => {
    await new Promise((resolve) => setTimeout(resolve));
    await fixture.whenStable();
  };
  await settle();
  return { fake, root: fixture.nativeElement as HTMLElement, settle };
}

function dialog(root: HTMLElement): HTMLDialogElement | null {
  return root.querySelector<HTMLDialogElement>('[data-testid="passkey-offer"]');
}

function press(root: HTMLElement, testId: string): void {
  root.querySelector<HTMLButtonElement>(`[data-testid="${testId}"]`)?.click();
}

describe('PasskeyOfferComponent', () => {
  afterEach(() => {
    localStorage.clear();
  });

  it('offers once, in the words of the Mac’s own authenticator, and makes the passkey', async () => {
    const { fake, root, settle } = await render();
    expect(dialog(root)?.open).toBe(true);
    expect(root.textContent).toContain('Sign in with Touch ID next time?');
    expect(root.querySelector('[data-testid="passkey-offer-make"]')?.textContent.trim()).toBe(
      'Use Touch ID',
    );
    await expectAccessible(root);
    press(root, 'passkey-offer-make');
    await settle();
    expect(fake.paramsOf('passkey/add')).toEqual([
      expect.objectContaining({ ceremony: 'c1' }) as unknown,
    ]);
    expect(dialog(root)?.open).toBe(false);
    expect(TestBed.inject(SessionService).offersPasskey()).toBe(false);
    expect(TestBed.inject(SessionService).passkeySignIn()).toBe(true);
  });

  it('says why when the device made none, and lets the person try again', async () => {
    const { root, settle } = await render({ answer: null });
    press(root, 'passkey-offer-make');
    await settle();
    expect(root.querySelector('[data-testid="passkey-offer-problem"]')?.textContent).toBe(
      passkeyRefused(),
    );
    expect(dialog(root)?.open).toBe(true);
    await expectAccessible(root);
  });

  it('remembers not now in this browser, and asks no more', async () => {
    const { root, settle } = await render();
    press(root, 'passkey-offer-later');
    await settle();
    expect(dialog(root)?.open).toBe(false);
    TestBed.resetTestingModule();
    const again = await render();
    expect(dialog(again.root)?.open).toBe(false);
  });

  it('offers nothing where a passkey exists, none can be made here, or no authenticator is', async () => {
    for (const setting of [
      { discovered: { ...here(), signin: ['link', 'passkey'] } },
      { discovered: { id: FAKE_SERVER, signin: ['link'] } },
      { discovered: { ...here(), here: 'http://localhost:1' } },
      { available: false },
    ]) {
      TestBed.resetTestingModule();
      const { root } = await render(setting);
      expect(dialog(root)?.open, JSON.stringify(setting)).toBe(false);
    }
  });
});
