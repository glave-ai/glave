import { filterOptions } from './options';

const MODELS = [
  { value: 'hope', label: 'Hope' },
  { value: 'cafe', label: 'Café model' },
  { value: 'big', label: 'A bigger model' },
];

describe('options', () => {
  it('filters by label, ignoring case and accents', () => {
    expect(filterOptions(MODELS, 'MODEL').map((option) => option.value)).toEqual(['cafe', 'big']);
    expect(filterOptions(MODELS, 'cafe').map((option) => option.value)).toEqual(['cafe']);
    expect(filterOptions(MODELS, '  ')).toBe(MODELS);
  });

  it('finds an option by the hint under it too', () => {
    const hinted = [
      { value: 'a', label: 'Claude Haiku 5.5', hint: 'OpenRouter · 1M context' },
      { value: 'b', label: 'Gemma 4', hint: 'Ollama' },
    ];
    expect(filterOptions(hinted, 'openrouter').map((option) => option.value)).toEqual(['a']);
    expect(filterOptions(hinted, 'gemma').map((option) => option.value)).toEqual(['b']);
  });
});
