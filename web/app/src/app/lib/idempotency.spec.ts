import { idempotencyKey } from './idempotency';

describe('idempotencyKey', () => {
  it('is 128 random bits as hex, a new one each time', () => {
    const [one, two] = [idempotencyKey(), idempotencyKey()];
    expect([one, two].every((key) => /^[0-9a-f]{32}$/u.test(key))).toBe(true);
    expect(one).not.toBe(two);
  });
});
