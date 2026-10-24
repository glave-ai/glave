import type { Mock } from 'vitest';

import { signalAccepted, signalUnknown, unknownPasskey } from './signals';

const ACCEPTED = { party: 'glave.test', user: 'AQID', credentials: ['BAU', 'Bgc'] };
const UNKNOWN = { party: 'localhost', credential: 'CAk' };

/** A stand-in for the browser's `PublicKeyCredential` with the Signal API. */
interface StubSignals {
  readonly signalAllAcceptedCredentials: Mock<() => Promise<void>>;
  readonly signalUnknownCredential: Mock<() => Promise<void>>;
}

/** A browser's `PublicKeyCredential` with the Signal API, recording what it was told. */
function withSignals(): StubSignals {
  return {
    signalAllAcceptedCredentials: vi.fn(() => Promise.resolve()),
    signalUnknownCredential: vi.fn(() => Promise.resolve()),
  };
}

describe('signals', () => {
  it("tells the passkey manager every passkey that signs the person in, as the API's names", async () => {
    const credential = withSignals();
    await signalAccepted(credential, ACCEPTED);
    expect(credential.signalAllAcceptedCredentials).toHaveBeenCalledWith({
      rpId: 'glave.test',
      userId: 'AQID',
      allAcceptedCredentialIds: ['BAU', 'Bgc'],
    });
    expect(credential.signalUnknownCredential).not.toHaveBeenCalled();
  });

  it('tells the passkey manager of a passkey the server does not know', async () => {
    const credential = withSignals();
    await signalUnknown(credential, UNKNOWN);
    expect(credential.signalUnknownCredential).toHaveBeenCalledWith({
      rpId: 'localhost',
      credentialId: 'CAk',
    });
    expect(credential.signalAllAcceptedCredentials).not.toHaveBeenCalled();
  });

  it('sends nothing and fails nothing where the browser has no Signal API', async () => {
    await expect(signalAccepted({}, ACCEPTED)).resolves.toBeUndefined();
    await expect(signalUnknown({}, UNKNOWN)).resolves.toBeUndefined();
    await expect(signalAccepted(null, ACCEPTED)).resolves.toBeUndefined();
    await expect(signalUnknown(null, UNKNOWN)).resolves.toBeUndefined();
  });

  it('fails nothing when the browser refuses a signal', async () => {
    const refusing = {
      signalAllAcceptedCredentials: () => Promise.reject(new DOMException('no', 'SecurityError')),
      signalUnknownCredential: () => {
        throw new TypeError('not base64url');
      },
    };
    await expect(signalAccepted(refusing, ACCEPTED)).resolves.toBeUndefined();
    await expect(signalUnknown(refusing, UNKNOWN)).resolves.toBeUndefined();
  });

  it("reads the passkey a refusal's data names as unknown, and nothing else", () => {
    expect(unknownPasskey({ unknown_passkey: UNKNOWN })).toEqual(UNKNOWN);
    expect(unknownPasskey({ failure: 'refused' })).toBeNull();
    expect(unknownPasskey({ unknown_passkey: { party: 'localhost' } })).toBeNull();
    expect(unknownPasskey({ unknown_passkey: null })).toBeNull();
    expect(unknownPasskey(undefined)).toBeNull();
  });
});
