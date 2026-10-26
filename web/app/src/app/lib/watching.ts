import { ApiError, ErrorCode } from '@glave/client';

/** How long the first try again waits after a watch was cut off; each one after, twice that. */
export const FIRST_RETRY_MS = 1_000;
/** The longest a try again waits. */
export const LAST_RETRY_MS = 30_000;

/**
 * Whether a watch that failed with `error` was cut off, and comes back: not when the tab was
 * signed out, or what it watched is not there.
 */
export function cutOff(error: unknown): boolean {
  return !(error instanceof ApiError && (error.signedOut || error.code === ErrorCode.NO_THREAD));
}

/**
 * Whether a request that failed with `error` was refused, the server saying why: not cut short,
 * nor answered with what could not be read, which may have reached it all the same.
 */
export function refused(error: unknown): error is ApiError {
  return (
    error instanceof ApiError && error.code !== ErrorCode.INTERNAL && error.code !== ErrorCode.PARSE
  );
}

/** Waits `ms` milliseconds, or until `signal` aborts. */
export function pause(ms: number, signal: AbortSignal): Promise<void> {
  return new Promise((resolve) => {
    const done = (): void => {
      clearTimeout(timer);
      signal.removeEventListener('abort', done);
      resolve();
    };
    const timer = setTimeout(done, ms);
    signal.addEventListener('abort', done);
  });
}
