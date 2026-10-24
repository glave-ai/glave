import { type Mock, vi } from 'vitest';

/** A stand-in for the browser's `PublicKeyCredential` with the Signal API. */
export interface StubPasskeySignals {
  readonly signalAllAcceptedCredentials: Mock<() => Promise<void>>;
  readonly signalUnknownCredential: Mock<() => Promise<void>>;
}

/** A browser's `PublicKeyCredential` with the Signal API, each signal heard and resolved. */
export function stubPasskeySignals(): StubPasskeySignals {
  return {
    signalAllAcceptedCredentials: vi.fn(() => Promise.resolve()),
    signalUnknownCredential: vi.fn(() => Promise.resolve()),
  };
}
