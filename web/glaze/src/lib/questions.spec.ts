import { composeAnswer, replaceAt, togglePick } from './questions';

describe('questions', () => {
  it('composes an answer from the choices and own words', () => {
    expect(composeAnswer(['Rust', 'Go'], '  and Zig ')).toBe('Rust, Go, and Zig');
    expect(composeAnswer([], 'my own')).toBe('my own');
    expect(composeAnswer([], ' ')).toBe('');
  });

  it('toggles a pick in and out', () => {
    expect(togglePick(['a'], 'b', true)).toEqual(['a', 'b']);
    expect(togglePick(['a', 'b'], 'a', false)).toEqual(['b']);
    expect(togglePick(['a'], 'a', true)).toEqual(['a']);
  });

  it('replaces one item', () => {
    expect(replaceAt(['a', 'b', 'c'], 1, 'x')).toEqual(['a', 'x', 'c']);
  });
});
