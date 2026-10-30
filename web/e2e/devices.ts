import type { BrowserContext, CDPSession, Page } from '@playwright/test';

/**
 * Devices that make and use passkeys in end-to-end tests: Chromium's virtual authenticator, or
 * in WebKit, which offers tests none, a scripted one in the page. Shared by the tests that
 * sign in with passkeys against a real glave.
 */

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
export interface Carried {
  readonly credentials: Awaited<ReturnType<typeof heldBy>>;
  readonly key: string | null;
}

/** One device's tabs, and what its passkeys are to carry to another browser. */
export interface Device {
  readonly tab: () => Promise<Page>;
  readonly carried: () => Promise<Carried>;
}

/**
 * Opens tabs on one device, each able to make and use its passkeys, those `carried` from
 * another browser first: in Chromium a virtual authenticator per tab, holding the passkeys the
 * tabs before it made; in WebKit the script, whose key every tab reads from the same storage.
 */
export function oneDevice(
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
