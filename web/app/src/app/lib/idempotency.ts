import { ApiError, ErrorCode } from '@glave/client';

/** A send the server may not have had: its text, and the idempotency key it went with. */
export interface Unsent {
  readonly prompt: string;
  readonly key: string;
}

/**
 * A new idempotency key for a send, made as the person sends it: 128 random bits, as hex. From
 * `crypto.getRandomValues`, which a page served over plain HTTP has too, where `randomUUID`
 * needs a secure context that a person's own server, reached at its address, may not give.
 */
export function idempotencyKey(): string {
  const bytes = crypto.getRandomValues(new Uint8Array(16));
  return Array.from(bytes, (byte) => byte.toString(16).padStart(2, '0')).join('');
}

/**
 * `unsent` when `prompt` is its text sent again: the same send, which goes with its key, so the
 * server starts its turn once. Otherwise `null`: a send of its own.
 */
export function sameSend<T extends Unsent>(prompt: string, unsent: T | null): T | null {
  return unsent?.prompt === prompt ? unsent : null;
}

/**
 * Whether `error` leaves it unknown whether the server had the send: the connection failed, its
 * answer never came whole, or the server failed of itself. Sending it again with its key is safe
 * then, as the server forgets the key of a turn that never started. An answer that says why the
 * send was refused is no such error, and the next send is a new one.
 */
export function answerLost(error: unknown): boolean {
  return (
    !(error instanceof ApiError) ||
    error.code === ErrorCode.INTERNAL ||
    error.code === ErrorCode.PARSE
  );
}
