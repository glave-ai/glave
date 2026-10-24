/**
 * WebAuthn's Signal API (`specs/session.md`, "Signing in"): the browser tells its passkey
 * manager what the server says of the person's passkeys, every one that signs them in after a
 * sign-in and the one it does not know when it refuses one, so the manager hides or deletes the
 * dead ones itself. Where the browser has no such signal, nothing is sent and nothing fails.
 */

import type { AcceptedPasskeys, UnknownPasskey } from '@glave/client';

/** The signals the browser's `PublicKeyCredential` gives, where it gives them. */
export type PasskeySignals = Partial<
  Pick<typeof PublicKeyCredential, 'signalAllAcceptedCredentials' | 'signalUnknownCredential'>
>;

/** Tells the passkey manager behind `signals` which of the person's passkeys sign them in. */
export async function signalAccepted(
  signals: PasskeySignals | null,
  accepted: AcceptedPasskeys,
): Promise<void> {
  await quietly(() =>
    signals?.signalAllAcceptedCredentials?.({
      rpId: accepted.party,
      userId: accepted.user,
      allAcceptedCredentialIds: [...accepted.credentials],
    }),
  );
}

/** Tells the passkey manager behind `signals` of a passkey the server does not know. */
export async function signalUnknown(
  signals: PasskeySignals | null,
  unknown: UnknownPasskey,
): Promise<void> {
  await quietly(() =>
    signals?.signalUnknownCredential?.({ rpId: unknown.party, credentialId: unknown.credential }),
  );
}

/** The passkey a refusal's `data` names as one the server does not know, if it names one. */
export function unknownPasskey(data: unknown): UnknownPasskey | null {
  if (typeof data !== 'object' || data === null || !('unknown_passkey' in data)) {
    return null;
  }
  const named: unknown = data.unknown_passkey;
  if (typeof named !== 'object' || named === null) {
    return null;
  }
  const { party, credential } = named as Partial<Record<keyof UnknownPasskey, unknown>>;
  return typeof party === 'string' && typeof credential === 'string' ? { party, credential } : null;
}

/** Runs `signal`; a passkey manager that refuses or fails it changes nothing the app does. */
async function quietly(signal: () => Promise<void> | undefined): Promise<void> {
  try {
    await signal();
  } catch {
    // The browser said no (another origin's relying party, say): the server stays the record.
  }
}
