import { isLong } from './text';

describe('isLong', () => {
  it('is long past either limit', () => {
    expect(isLong('short', 10, 2)).toBe(false);
    expect(isLong('x'.repeat(11), 10, 2)).toBe(true);
    expect(isLong('a\nb\nc', 10, 2)).toBe(true);
    expect(isLong('a\nb', 10, 2)).toBe(false);
  });
});
