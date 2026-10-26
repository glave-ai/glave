import { ApiError, ErrorCode } from '@glave/client';

import { cutOff, pause, refused } from './watching';

describe('watching', () => {
  it('comes back from a watch cut off, not once signed out or with nothing to watch', () => {
    expect(cutOff(new TypeError('Failed to fetch'))).toBe(true);
    expect(cutOff(new ApiError({ code: ErrorCode.INTERNAL, message: 'the server failed' }))).toBe(
      true,
    );
    expect(cutOff(new ApiError({ code: ErrorCode.NO_SIGN_IN, message: 'signed out' }))).toBe(false);
    expect(cutOff(new ApiError({ code: ErrorCode.NO_THREAD, message: 'no thread' }))).toBe(false);
  });

  it('tells a refusal, where the server said why, from an answer that never came', () => {
    expect(refused(new ApiError({ code: ErrorCode.QUEUE_FULL, message: 'too many' }))).toBe(true);
    expect(refused(new ApiError({ code: ErrorCode.INTERNAL, message: 'cut off' }))).toBe(false);
    expect(refused(new ApiError({ code: ErrorCode.PARSE, message: 'not JSON-RPC' }))).toBe(false);
    expect(refused(new TypeError('Load failed'))).toBe(false);
  });

  it('waits its time, or until it is stopped', async () => {
    vi.useFakeTimers();
    try {
      let waited = false;
      const waiting = pause(1_000, new AbortController().signal).then(() => {
        waited = true;
      });
      await vi.advanceTimersByTimeAsync(999);
      expect(waited).toBe(false);
      await vi.advanceTimersByTimeAsync(1);
      await waiting;
      expect(waited).toBe(true);
      const stop = new AbortController();
      const stopped = pause(60_000, stop.signal);
      stop.abort();
      await stopped;
    } finally {
      vi.useRealTimers();
    }
  });
});
