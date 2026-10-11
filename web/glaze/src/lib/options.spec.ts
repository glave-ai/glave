import { filterOptions, labelOf } from './options';

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

  it('finds a label by value', () => {
    expect(labelOf(MODELS, 'hope')).toBe('Hope');
    expect(labelOf(MODELS, null)).toBe('');
  });
});
