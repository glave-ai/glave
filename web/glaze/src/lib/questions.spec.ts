import { composeAnswer, replaceAt, sameQuestions, togglePick } from './questions';

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

  it('tells questions that ask the same from ones that ask something else', () => {
    const options = [{ label: 'A', description: 'first' }];
    const asked = [{ question: 'Which?', options }];
    const again = [{ question: 'Which?', options: [{ label: 'A', description: 'first' }] }];
    expect(sameQuestions(asked, again)).toBe(true);
    expect(sameQuestions(asked, [{ question: 'Which?', options: [{ label: 'A' }] }])).toBe(false);
    expect(sameQuestions(asked, [{ question: 'Why?', options }])).toBe(false);
    expect(sameQuestions(asked, [{ question: 'Which?', options, multiple: true }])).toBe(false);
    expect(sameQuestions(asked, [...again, { question: 'And?' }])).toBe(false);
  });

  it('replaces one item', () => {
    expect(replaceAt(['a', 'b', 'c'], 1, 'x')).toEqual(['a', 'x', 'c']);
  });
});
