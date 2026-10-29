import { type BrowserContext, type CDPSession, type Page } from '@playwright/test';

import {
  ADMIN_WORDS,
  HERE,
  HERE_WEBKIT,
  PASSKEYS,
  PASSKEYS_ORIGIN,
  byTestId,
  expect,
  expectAccessible,
  heldUntilHeard,
  served,
  setupCode,
  signInCode,
  test,
  watchForProblems,
} from './support';

/** Words a person on their own server should never meet, nor a command's flags. */
const ADMIN_OR_FLAGS = new RegExp(`${ADMIN_WORDS.source}|--`, 'iu');

/**
 * Sends every request to the public origin on to the real glave, as a tunnel on its computer
 * does: the page stays at `https://glave.test`, a secure origin, WebAuthn's relying party.
 */
async function tunnel(context: BrowserContext): Promise<void> {
  const glave = served(PASSKEYS);
  await context.route(`${PASSKEYS_ORIGIN}/**`, async (route) => {
    // A watch streams for as long as the page lives, which a tunnel of whole answers cannot
    // carry: it is left waiting, as a quiet one would be.
    if (route.request().headers()['glave-method']?.endsWith('/watch') === true) {
      return;
    }
    const asked = new URL(route.request().url());
    const response = await route.fetch({
      url: `${glave}${asked.pathname}${asked.search}`,
      headers: { ...route.request().headers(), host: new URL(PASSKEYS_ORIGIN).host },
    });
    await route.fulfill({ response });
  });
}

/**
 * A passkey authenticator in the page, for WebKit, which offers tests no virtual one: a P-256
 * key kept in the page's storage, attestation `none`, the person verified, a counter. Its
 * answers are shaped as the browser's are.
 */
function scriptedAuthenticator(): void {
  const KEPT = 'glave-e2e-authenticator';
  const encoder = new TextEncoder();
  const toBase64url = (bytes: Uint8Array): string =>
    btoa(String.fromCharCode(...bytes))
      .replaceAll('+', '-')
      .replaceAll('/', '_')
      .replace(/=+$/u, '');
  const fromBase64url = (text: string): Uint8Array =>
    Uint8Array.from(atob(text.replaceAll('-', '+').replaceAll('_', '/')), (c) => c.charCodeAt(0));
  const text = (value: string): number[] => [0x60 | value.length, ...encoder.encode(value)];
  const bytes = (value: Uint8Array): number[] => [
    0x59,
    value.length >> 8,
    value.length & 0xff,
    ...value,
  ];
  interface Kept {
    readonly key: JsonWebKey;
    readonly id: string;
    readonly counter: number;
  }
  const read = (): Kept | null => {
    const kept = localStorage.getItem(KEPT);
    return kept === null ? null : (JSON.parse(kept) as Kept);
  };
  const data = async (party: string, flags: number, counter: number): Promise<number[]> => {
    const hash = new Uint8Array(await crypto.subtle.digest('SHA-256', encoder.encode(party)));
    return [
      ...hash,
      flags,
      counter >>> 24,
      (counter >>> 16) & 0xff,
      (counter >>> 8) & 0xff,
      counter & 0xff,
    ];
  };
  const client = (type: string, challenge: BufferSource): Uint8Array<ArrayBuffer> =>
    encoder.encode(
      JSON.stringify({
        type,
        challenge: toBase64url(new Uint8Array(challenge as ArrayBuffer)),
        origin: location.origin,
        crossOrigin: false,
      }),
    );
  const integer = (raw: Uint8Array): number[] => {
    let value = Array.from(raw);
    while (value.length > 1 && value[0] === 0) {
      value = value.slice(1);
    }
    if ((value[0] ?? 0) & 0x80) {
      value = [0, ...value];
    }
    return [0x02, value.length, ...value];
  };
  const credentials = {
    async create(options: CredentialCreationOptions): Promise<unknown> {
      const asked = options.publicKey;
      if (asked === undefined) {
        return null;
      }
      const pair = await crypto.subtle.generateKey({ name: 'ECDSA', namedCurve: 'P-256' }, true, [
        'sign',
      ]);
      const point = new Uint8Array(await crypto.subtle.exportKey('raw', pair.publicKey));
      const id = crypto.getRandomValues(new Uint8Array(16));
      const key = [0xa5, 0x01, 0x02, 0x03, 0x26, 0x20, 0x01, 0x21, 0x58, 0x20];
      key.push(...point.slice(1, 33), 0x22, 0x58, 0x20, ...point.slice(33, 65));
      const made = await data(asked.rp.id ?? location.hostname, 0x45, 1);
      made.push(...new Array<number>(16).fill(0), 0, 16, ...id, ...key);
      const attestation = [0xa3, ...text('fmt'), ...text('none'), ...text('attStmt'), 0xa0];
      attestation.push(...text('authData'), ...bytes(Uint8Array.from(made)));
      const kept: Kept = {
        key: await crypto.subtle.exportKey('jwk', pair.privateKey),
        id: toBase64url(id),
        counter: 1,
      };
      localStorage.setItem(KEPT, JSON.stringify(kept));
      return {
        id: kept.id,
        rawId: id.buffer,
        type: 'public-key',
        response: {
          attestationObject: Uint8Array.from(attestation).buffer,
          clientDataJSON: client('webauthn.create', asked.challenge).buffer,
          getTransports: () => ['internal'],
        },
        getClientExtensionResults: () => ({}),
      };
    },
    async get(options: CredentialRequestOptions): Promise<unknown> {
      const asked = options.publicKey;
      const kept = read();
      if (asked === undefined || kept === null) {
        return null;
      }
      const counter = kept.counter + 1;
      localStorage.setItem(KEPT, JSON.stringify({ ...kept, counter }));
      const key = await crypto.subtle.importKey(
        'jwk',
        kept.key,
        { name: 'ECDSA', namedCurve: 'P-256' },
        false,
        ['sign'],
      );
      const used = Uint8Array.from(await data(asked.rpId ?? location.hostname, 0x05, counter));
      const clientData = client('webauthn.get', asked.challenge);
      const hash = new Uint8Array(await crypto.subtle.digest('SHA-256', clientData));
      const signed = new Uint8Array(
        await crypto.subtle.sign(
          { name: 'ECDSA', hash: 'SHA-256' },
          key,
          Uint8Array.from([...used, ...hash]),
        ),
      );
      const r = integer(signed.slice(0, 32));
      const s = integer(signed.slice(32));
      return {
        id: kept.id,
        rawId: fromBase64url(kept.id).buffer,
        type: 'public-key',
        response: {
          authenticatorData: used.buffer,
          clientDataJSON: clientData.buffer,
          signature: Uint8Array.from([0x30, r.length + s.length, ...r, ...s]).buffer,
          userHandle: null,
        },
        getClientExtensionResults: () => ({}),
      };
    },
  };
  Object.defineProperty(navigator, 'credentials', { value: credentials, configurable: true });
  // An authenticator of the computer's own, which checks who is there. WebKit as built for
  // Linux has no WebAuthn, so no `PublicKeyCredential` either, which the script then stands in
  // for, as it does for `navigator.credentials`.
  const available = (): Promise<boolean> => Promise.resolve(true);
  if (typeof PublicKeyCredential === 'function') {
    PublicKeyCredential.isUserVerifyingPlatformAuthenticatorAvailable = available;
  } else {
    Object.defineProperty(window, 'PublicKeyCredential', {
      value: { isUserVerifyingPlatformAuthenticatorAvailable: available },
      configurable: true,
    });
  }
}

/** Where the scripted authenticator keeps its key, in the page's storage. */
const SCRIPTED = 'glave-e2e-authenticator';

/** A virtual authenticator Chromium gives one page, through the DevTools protocol. */
interface Virtual {
  readonly cdp: CDPSession;
  readonly authenticatorId: string;
}

/** The passkeys `virtual` holds. */
// eslint-disable-next-line @typescript-eslint/explicit-function-return-type -- the protocol's own credentials, whose type Playwright does not export
async function heldBy(virtual: Virtual) {
  const { authenticatorId } = virtual;
  return (await virtual.cdp.send('WebAuthn.getCredentials', { authenticatorId })).credentials;
}

/**
 * A device's passkeys, carried to another browser on the same computer, as the system keeps
 * them for every browser: Chromium's virtual credentials, or the scripted authenticator's key.
 */
interface Carried {
  readonly credentials: Awaited<ReturnType<typeof heldBy>>;
  readonly key: string | null;
}

/** One device's tabs, and what its passkeys are to carry to another browser. */
interface Device {
  readonly tab: () => Promise<Page>;
  readonly carried: () => Promise<Carried>;
}

/**
 * Opens tabs on one device, each able to make and use its passkeys, those `carried` from
 * another browser first: in Chromium a virtual authenticator per tab, holding the passkeys the
 * tabs before it made; in WebKit the script, whose key every tab reads from the same storage.
 */
function oneDevice(
  context: BrowserContext,
  browserName: string,
  carried: Carried = { credentials: [], key: null },
): Device {
  let before: Virtual | null = null;
  let last: Page | null = null;
  const tab = async (): Promise<Page> => {
    const page = await context.newPage();
    last = page;
    if (browserName !== 'chromium') {
      await page.addInitScript(scriptedAuthenticator);
      if (carried.key !== null) {
        await page.addInitScript(
          ([name, key]) => {
            if (localStorage.getItem(name) === null) {
              localStorage.setItem(name, key);
            }
          },
          [SCRIPTED, carried.key] as const,
        );
      }
      return page;
    }
    const cdp = await context.newCDPSession(page);
    await cdp.send('WebAuthn.enable');
    const { authenticatorId } = await cdp.send('WebAuthn.addVirtualAuthenticator', {
      options: {
        protocol: 'ctap2',
        transport: 'internal',
        hasResidentKey: true,
        hasUserVerification: true,
        isUserVerified: true,
        automaticPresenceSimulation: true,
      },
    });
    const made = before === null ? carried.credentials : await heldBy(before);
    for (const credential of made) {
      await cdp.send('WebAuthn.addCredential', { authenticatorId, credential });
    }
    before = { cdp, authenticatorId };
    return page;
  };
  return {
    tab,
    carried: async () => ({
      credentials: before === null ? [] : await heldBy(before),
      key:
        last === null ? null : await last.evaluate((name) => localStorage.getItem(name), SCRIPTED),
    }),
  };
}

/**
 * Signing in from elsewhere with passkeys against a real glave reached at a public origin
 * (`specs/session.md`, "Signing in"): the setup link glave on its computer makes opens a page
 * that makes a passkey on this device and signs the browser in, the link working once; a new
 * tab is signed in with it, and once signed out the passkey signs it in again, and Settings
 * lists it.
 */
test('a setup link makes a passkey here, and the passkey signs the browser in again', async ({
  context,
  browser,
  browserName,
}) => {
  await tunnel(context);
  const { tab } = oneDevice(context, browserName);
  const code = await setupCode(PASSKEYS);

  const page = await tab();
  const problems = watchForProblems(page);
  await page.goto(`${PASSKEYS_ORIGIN}/#setup=${code}`);
  await expect(byTestId(page, 'sign-in')).toContainText('Make a passkey for this device');
  expect(page.url()).not.toContain(code);
  for (const colorScheme of ['light', 'dark'] as const) {
    await page.emulateMedia({ colorScheme });
    await expectAccessible(page);
  }
  expect(await page.locator('body').innerText()).not.toMatch(ADMIN_OR_FLAGS);
  await byTestId(page, 'sign-in-passkey').click();
  await expect(byTestId(page, 'home-composer-input')).toBeVisible();

  // A new tab of the browser is signed in. Signed out, it signs in again with the passkey, and
  // Settings lists it (Chromium's run and WebKit's each add one to the same glave).
  const again = await tab();
  const problemsAgain = watchForProblems(again);
  await again.goto(`${PASSKEYS_ORIGIN}/settings`);
  await byTestId(again, 'settings-sign-out').click();
  await expect(byTestId(again, 'sign-in')).toContainText('Sign in to glave');
  await expect(byTestId(page, 'sign-in')).toBeVisible();
  await expectAccessible(again);
  await byTestId(again, 'sign-in-passkey').click();
  await expect(byTestId(again, 'home-composer-input')).toBeVisible();
  await byTestId(again, 'person').click();
  await byTestId(again, 'nav-settings').click();
  await expect(byTestId(again, 'passkeys-passkey').first()).toBeVisible();
  await expectAccessible(again);

  // The setup link is spent, for another browser too.
  const another = await browser.newContext();
  await tunnel(another);
  const spent = await oneDevice(another, browserName).tab();
  const problemsSpent = watchForProblems(spent);
  await spent.goto(`${PASSKEYS_ORIGIN}/#setup=${code}`);
  await byTestId(spent, 'sign-in-passkey').click();
  await expect(byTestId(spent, 'sign-in-problem')).toBeVisible();
  expect([...problems(), ...problemsAgain(), ...problemsSpent()]).toEqual([]);
  await another.close();
});

/**
 * A passkey on this computer, against a real glave reached only here (`specs/session.md`,
 * "Signing in"): before any is made, a browser's sign-in page offers none; signed in by glave's
 * link at `localhost`, the app offers once to make one, in the words of the computer's own
 * authenticator; then another browser on this computer, which the system gives the same
 * passkeys, signs in with it in one step. Each engine has a glave of its own, as each makes the
 * first passkey.
 */
test('a passkey made here signs another browser on this computer in', async ({
  browser,
  browserName,
}) => {
  const site = browserName === 'webkit' ? HERE_WEBKIT : HERE;
  const here = `http://localhost:${new URL(served(site)).port}`;
  const nobody = await heldUntilHeard(await browser.newContext());
  const signedOut = await nobody.newPage();
  await signedOut.goto(`${here}/`);
  await expect(byTestId(signedOut, 'sign-in')).toBeVisible();
  await expect(byTestId(signedOut, 'sign-in-passkey')).toHaveCount(0);
  await nobody.close();

  const first = await heldUntilHeard(await browser.newContext());
  const device = oneDevice(first, browserName);
  const page = await device.tab();
  const problems = watchForProblems(page);
  await page.goto(`${here}/#code=${await signInCode(site)}`);
  const offer = byTestId(page, 'passkey-offer');
  await expect(offer).toBeVisible();
  await expect(offer).toContainText(/Sign in with (Touch ID|Windows Hello|a passkey) next time\?/u);
  for (const colorScheme of ['light', 'dark'] as const) {
    await page.emulateMedia({ colorScheme });
    await expectAccessible(page);
  }
  await byTestId(page, 'passkey-offer-make').click();
  await expect(offer).toBeHidden();
  await expect(byTestId(page, 'home-composer-input')).toBeVisible();

  const second = await heldUntilHeard(await browser.newContext());
  const other = await oneDevice(second, browserName, await device.carried()).tab();
  const problemsThere = watchForProblems(other);
  await other.goto(`${here}/`);
  const signIn = byTestId(other, 'sign-in-passkey');
  await expect(signIn).toHaveText(/Sign in with (Touch ID|Windows Hello|a passkey)/u);
  await expectAccessible(other);
  await signIn.click();
  await expect(byTestId(other, 'home-composer-input')).toBeVisible();
  await expect(byTestId(other, 'passkey-offer')).toBeHidden();
  expect([...problems(), ...problemsThere()]).toEqual([]);
  await second.close();
  await first.close();
});
