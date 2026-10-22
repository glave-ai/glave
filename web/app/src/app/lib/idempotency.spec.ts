import { idempotencyKey } from './idempotency';

describe('idempotencyKey', () => {
  it('is 128 random bits as hex, a new one each time', () => {
    const [one, two] = [idempotencyKey(), idempotencyKey()];
    expect(one).toMatch(/^[0-9a-f]{32}$/u);
    expect(two).toMatch(/^[0-9a-f]{32}$/u);
    expect(one).not.toBe(two);
  });
});
