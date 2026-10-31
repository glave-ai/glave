/**
 * Passkeys between glave and the browser (`specs/session.md`, "Signing in"): glave sends
 * WebAuthn's options as JSON, its buffers in base64url, and takes the browser's answer the same
 * way. These turn one into what `navigator.credentials` takes, and its answer back.
 */

/** `bytes` in base64url, without padding. */
export function toBase64url(bytes: ArrayBuffer | ArrayBufferView): string {
  const view =
    bytes instanceof ArrayBuffer
      ? new Uint8Array(bytes)
      : new Uint8Array(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  let binary = '';
  for (const byte of view) {
    binary += String.fromCharCode(byte);
  }
  return btoa(binary).replaceAll('+', '-').replaceAll('/', '_').replace(/=+$/u, '');
}

/** The bytes `text`, base64url with or without padding, stands for; `null` when it is not. */
export function fromBase64url(text: string): Uint8Array<ArrayBuffer> | null {
  try {
    const binary = atob(text.replaceAll('-', '+').replaceAll('_', '/'));
    const bytes = new Uint8Array(binary.length);
    for (let at = 0; at < binary.length; at += 1) {
      bytes[at] = binary.charCodeAt(at);
    }
    return bytes;
  } catch {
    return null;
  }
}

type Json = Readonly<Record<string, unknown>>;

function isJson(value: unknown): value is Json {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

/** The `publicKey` options of glave's JSON `options`, `null` when they are not that. */
function publicKeyOf(options: unknown): Json | null {
  return isJson(options) && isJson(options['publicKey']) ? options['publicKey'] : null;
}

/** Each credential `{type, id}` of `listed`, its id as bytes. */
function credentialsOf(listed: unknown): PublicKeyCredentialDescriptor[] {
  if (!Array.isArray(listed)) {
    return [];
  }
  return listed.flatMap((entry: unknown) => {
    const id = isJson(entry) && typeof entry['id'] === 'string' ? fromBase64url(entry['id']) : null;
    return id === null ? [] : [{ type: 'public-key' as const, id }];
  });
}

/** What `navigator.credentials.create` takes for glave's options to make a passkey. */
export function creationOptions(options: unknown): CredentialCreationOptions | null {
  const publicKey = publicKeyOf(options);
  const user = publicKey !== null && isJson(publicKey['user']) ? publicKey['user'] : null;
  const challenge =
    typeof publicKey?.['challenge'] === 'string' ? fromBase64url(publicKey['challenge']) : null;
  const userId = typeof user?.['id'] === 'string' ? fromBase64url(user['id']) : null;
  if (publicKey === null || user === null || challenge === null || userId === null) {
    return null;
  }
  return {
    publicKey: {
      ...(publicKey as unknown as PublicKeyCredentialCreationOptions),
      challenge,
      user: { ...(user as unknown as PublicKeyCredentialUserEntity), id: userId },
      excludeCredentials: credentialsOf(publicKey['excludeCredentials']),
    },
  };
}

/** What `navigator.credentials.get` takes for glave's options to sign in. */
export function requestOptions(options: unknown): CredentialRequestOptions | null {
  const publicKey = publicKeyOf(options);
  const challenge =
    typeof publicKey?.['challenge'] === 'string' ? fromBase64url(publicKey['challenge']) : null;
  if (publicKey === null || challenge === null) {
    return null;
  }
  return {
    publicKey: {
      ...(publicKey as unknown as PublicKeyCredentialRequestOptions),
      challenge,
      allowCredentials: credentialsOf(publicKey['allowCredentials']),
    },
  };
}

/** What makes and uses passkeys: the browser's `navigator.credentials`, or a test's stand-in. */
export interface Authenticator {
  create(options: CredentialCreationOptions): Promise<Credential | null>;
  get(options: CredentialRequestOptions): Promise<Credential | null>;
}

/**
 * The answer `authenticator` gives glave's `options` to make a passkey, as glave takes it;
 * `null` when it made none.
 */
export async function made(
  authenticator: Authenticator | null,
  options: unknown,
): Promise<Json | null> {
  const asked = creationOptions(options);
  const answer = asked === null ? null : await answered(() => authenticator?.create(asked));
  return isPublicKeyCredential(answer) ? createdJson(answer) : null;
}

/**
 * The answer `authenticator` gives glave's `options` to sign in with a passkey, as glave takes
 * it; `null` when it used none.
 */
export async function used(
  authenticator: Authenticator | null,
  options: unknown,
): Promise<Json | null> {
  const asked = requestOptions(options);
  const answer = asked === null ? null : await answered(() => authenticator?.get(asked));
  return isPublicKeyCredential(answer) ? assertedJson(answer) : null;
}

/** What the authenticator answers `ask` with; `null` when the person or the browser refused. */
async function answered(
  ask: () => Promise<Credential | null> | undefined,
): Promise<Credential | null> {
  try {
    return (await ask()) ?? null;
  } catch {
    return null;
  }
}

/** Whether `value`, what the browser's authenticator answered, is a passkey's answer. */
export function isPublicKeyCredential(value: unknown): value is PublicKeyCredential {
  return typeof value === 'object' && value !== null && 'rawId' in value && 'response' in value;
}

/** A new passkey's answer, as glave takes it. */
export function createdJson(credential: PublicKeyCredential): Json {
  const response = credential.response as AuthenticatorAttestationResponse;
  return {
    id: credential.id,
    rawId: toBase64url(credential.rawId),
    type: credential.type,
    response: {
      attestationObject: toBase64url(response.attestationObject),
      clientDataJSON: toBase64url(response.clientDataJSON),
      transports: response.getTransports(),
    },
    extensions: credential.getClientExtensionResults(),
  };
}

/** A passkey's answer to sign in, as glave takes it. */
export function assertedJson(credential: PublicKeyCredential): Json {
  const response = credential.response as AuthenticatorAssertionResponse;
  return {
    id: credential.id,
    rawId: toBase64url(credential.rawId),
    type: credential.type,
    response: {
      authenticatorData: toBase64url(response.authenticatorData),
      clientDataJSON: toBase64url(response.clientDataJSON),
      signature: toBase64url(response.signature),
      userHandle: response.userHandle === null ? null : toBase64url(response.userHandle),
    },
    extensions: credential.getClientExtensionResults(),
  };
}
