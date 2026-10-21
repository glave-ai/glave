import { isLong, sentence, somethingWrong } from './text';

describe('sentence', () => {
  it('starts with a capital and ends with a stop', () => {
    expect(sentence('glave could not reach it')).toBe('Glave could not reach it.');
    expect(sentence('Done!')).toBe('Done!');
    expect(sentence('  ')).toBe(somethingWrong());
  });
});

describe('isLong', () => {
  it('is long past either limit', () => {
    expect(isLong('short', 10, 2)).toBe(false);
    expect(isLong('x'.repeat(11), 10, 2)).toBe(true);
    expect(isLong('a\nb\nc', 10, 2)).toBe(true);
    expect(isLong('a\nb', 10, 2)).toBe(false);
  });
});
