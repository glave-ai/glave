import {
  assertedJson,
  createdJson,
  creationOptions,
  fromBase64url,
  passkeysAt,
  requestOptions,
  toBase64url,
} from './passkeys';

const bytes = (...values: readonly number[]): Uint8Array<ArrayBuffer> => new Uint8Array(values);

/** Options to make a passkey, as glave sends them. */
const CREATE = {
  publicKey: {
    rp: { id: 'glave.test', name: 'glave' },
    user: { id: 'AQID', name: 'glave', displayName: 'glave' },
    challenge: '-_8',
    pubKeyCredParams: [{ type: 'public-key', alg: -7 }],
    excludeCredentials: [{ type: 'public-key', id: 'BAU' }],
    attestation: 'none',
  },
};

describe('passkeys', () => {
  it('are made at a name and never at an address', () => {
    for (const name of ['localhost', 'glave.test', 'box.tail1234.ts.net', '1.example']) {
      expect(passkeysAt(name)).toBe(true);
    }
    for (const address of ['127.0.0.1', '192.168.1.2', '[::1]', '[2001:db8::1]']) {
      expect(passkeysAt(address)).toBe(false);
    }
  });

  it('writes and reads base64url, padded or not, and refuses what is not', () => {
    expect(toBase64url(bytes(251, 255, 191))).toBe('-_-_');
    expect(toBase64url(bytes(1, 2, 3, 4).buffer)).toBe('AQIDBA');
    expect(Array.from(fromBase64url('-_-_') ?? [])).toEqual([251, 255, 191]);
    expect(Array.from(fromBase64url('AQIDBA==') ?? [])).toEqual([1, 2, 3, 4]);
    expect(fromBase64url('not base64!')).toBeNull();
  });

  it('turns options to make a passkey into what the browser takes', () => {
    const options = creationOptions(CREATE)?.publicKey;
    expect(options?.rp).toEqual({ id: 'glave.test', name: 'glave' });
    expect(Array.from(new Uint8Array(options?.challenge as ArrayBuffer))).toEqual([251, 255]);
    expect(Array.from(new Uint8Array(options?.user.id as ArrayBuffer))).toEqual([1, 2, 3]);
    const excluded = options?.excludeCredentials?.[0]?.id as ArrayBuffer;
    expect(Array.from(new Uint8Array(excluded))).toEqual([4, 5]);
    expect(creationOptions({})).toBeNull();
    expect(creationOptions({ publicKey: { ...CREATE.publicKey, user: {} } })).toBeNull();
  });

  it('turns options to sign in into what the browser takes', () => {
    const options = requestOptions({
      publicKey: {
        challenge: 'AQID',
        rpId: 'glave.test',
        allowCredentials: [{ type: 'public-key', id: 'BAU' }, { id: 7 }],
      },
    })?.publicKey;
    expect(options?.rpId).toBe('glave.test');
    expect(options?.allowCredentials?.length).toBe(1);
    expect(requestOptions({ publicKey: {} })).toBeNull();
    expect(requestOptions('nothing')).toBeNull();
  });

  it("writes the browser's answers as glave takes them", () => {
    const made = {
      id: 'BAU',
      rawId: bytes(4, 5).buffer,
      type: 'public-key',
      response: {
        attestationObject: bytes(1).buffer,
        clientDataJSON: bytes(2).buffer,
        getTransports: () => ['internal'],
      },
      getClientExtensionResults: () => ({}),
    } as unknown as PublicKeyCredential;
    expect(createdJson(made)).toEqual({
      id: 'BAU',
      rawId: 'BAU',
      type: 'public-key',
      response: { attestationObject: 'AQ', clientDataJSON: 'Ag', transports: ['internal'] },
      extensions: {},
    });
    const used = {
      id: 'BAU',
      rawId: bytes(4, 5).buffer,
      type: 'public-key',
      response: {
        authenticatorData: bytes(1).buffer,
        clientDataJSON: bytes(2).buffer,
        signature: bytes(3).buffer,
        userHandle: null,
      },
      getClientExtensionResults: () => ({}),
    } as unknown as PublicKeyCredential;
    expect(assertedJson(used)).toEqual({
      id: 'BAU',
      rawId: 'BAU',
      type: 'public-key',
      response: {
        authenticatorData: 'AQ',
        clientDataJSON: 'Ag',
        signature: 'Aw',
        userHandle: null,
      },
      extensions: {},
    });
  });
});
