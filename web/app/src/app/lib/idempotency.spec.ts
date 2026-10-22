import { ApiError, ErrorCode } from '@glave/client';

import { answerLost, idempotencyKey, sameSend } from './idempotency';

describe('idempotency', () => {
  it('makes a key of 128 random bits as hex, a new one each time', () => {
    const [one, two] = [idempotencyKey(), idempotencyKey()];
    expect(one).toMatch(/^[0-9a-f]{32}$/u);
    expect(two).toMatch(/^[0-9a-f]{32}$/u);
    expect(one).not.toBe(two);
  });

  it('takes the same text sent again for the same send, and other text for a new one', () => {
    const unsent = { prompt: 'Plan the launch', key: 'k1', thread: 't1' };
    expect(sameSend('Plan the launch', unsent)).toBe(unsent);
    expect(sameSend('Plan the launch party', unsent)).toBeNull();
    expect(sameSend('Plan the launch', null)).toBeNull();
  });

  it('counts an answer as lost when it never came whole, not when the server said why', () => {
    expect(answerLost(new TypeError('Failed to fetch'))).toBe(true);
    const ended = new ApiError({
      code: ErrorCode.INTERNAL,
      message: 'the stream ended without an answer',
    });
    expect(answerLost(ended)).toBe(true);
    expect(answerLost(new ApiError({ code: ErrorCode.PARSE, message: 'not JSON-RPC' }))).toBe(true);
    const refused = new ApiError({ code: ErrorCode.QUEUE_FULL, message: 'The server is stopping' });
    expect(answerLost(refused)).toBe(false);
  });
});
