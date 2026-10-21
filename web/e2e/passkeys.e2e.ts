import { type BrowserContext, type CDPSession, type Page, expect, test } from '@playwright/test';

import {
  PASSKEYS,
  PASSKEYS_ORIGIN,
  byTestId,
  expectAccessible,
  served,
  setupCode,
  watchForProblems,
} from './support';

/** Words a person on their own server should never meet (AGENTS.md, "Keeping principle 4"). */
const ADMIN_WORDS = /policy|organization|member|group|admin|SSO|config\.toml|GLAVE_|--/u;

/**
 * Sends every request to the public origin on to the real glave, as a tunnel on its computer
 * does: the page stays at `https://glave.test`, a secure origin, WebAuthn's relying party.
 */
async function tunnel(context: BrowserContext): Promise<void> {
  const glave = served(PASSKEYS);
  await context.route(`${PASSKEYS_ORIGIN}/**`, async (route) => {
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
}

/** A virtual authenticator Chromium gives one page, through the DevTools protocol. */
interface Virtual {
  readonly cdp: CDPSession;
  readonly authenticatorId: string;
}

/**
 * Opens tabs on one device, each able to make and use its passkeys: in Chromium a virtual
 * authenticator per tab, holding the passkeys the tabs before it made; in WebKit the script,
 * whose key every tab reads from the same storage.
 */
function oneDevice(context: BrowserContext, browserName: string): () => Promise<Page> {
  let before: Virtual | null = null;
  return async () => {
    const page = await context.newPage();
    if (browserName !== 'chromium') {
      await page.addInitScript(scriptedAuthenticator);
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
    const made =
      before === null
        ? []
        : (
            await before.cdp.send('WebAuthn.getCredentials', {
              authenticatorId: before.authenticatorId,
            })
          ).credentials;
    for (const credential of made) {
      await cdp.send('WebAuthn.addCredential', { authenticatorId, credential });
    }
    before = { cdp, authenticatorId };
    return page;
  };
}

/**
 * Signing in from elsewhere with passkeys against a real glave reached at a public origin
 * (`specs/session.md`, "Signing in"): the setup link glave on its computer makes opens a page
 * that makes a passkey on this device and signs the tab in, the link working once; then the
 * passkey signs a new tab in, and Settings lists it.
 */
test('a setup link makes a passkey here, and the passkey signs the browser in again', async ({
  context,
  browserName,
}) => {
  await tunnel(context);
  const tab = oneDevice(context, browserName);
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
  expect(await page.locator('body').innerText()).not.toMatch(ADMIN_WORDS);
  await byTestId(page, 'sign-in-passkey').click();
  await expect(byTestId(page, 'home-composer-input')).toBeVisible();

  // A new tab keeps no session: it signs in with the passkey, and Settings lists it (Chromium's
  // run and WebKit's each add one to the same glave).
  const again = await tab();
  const problemsAgain = watchForProblems(again);
  await again.goto(`${PASSKEYS_ORIGIN}/`);
  await expect(byTestId(again, 'sign-in')).toContainText('Sign in to glave');
  await expectAccessible(again);
  await byTestId(again, 'sign-in-passkey').click();
  await expect(byTestId(again, 'home-composer-input')).toBeVisible();
  await byTestId(again, 'person').click();
  await byTestId(again, 'nav-settings').click();
  await expect(byTestId(again, 'passkeys-passkey').first()).toBeVisible();
  await expectAccessible(again);

  // The setup link is spent.
  const spent = await tab();
  const problemsSpent = watchForProblems(spent);
  await spent.goto(`${PASSKEYS_ORIGIN}/#setup=${code}`);
  await byTestId(spent, 'sign-in-passkey').click();
  await expect(byTestId(spent, 'sign-in-problem')).toBeVisible();
  expect([...problems(), ...problemsAgain(), ...problemsSpent()]).toEqual([]);
});
