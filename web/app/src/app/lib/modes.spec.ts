import { modes } from './modes';

describe('modes', () => {
  it('names every mode, says what it means, gives it an icon, and recommends Auto alone', () => {
    const all = modes();
    expect(Object.values(all).map((mode) => mode.text)).toEqual([
      'Auto',
      'Plan',
      'Ask first',
      'Full access',
    ]);
    expect(Object.values(all).every((mode) => mode.about !== '' && mode.icon.length > 0)).toBe(
      true,
    );
    expect(Object.entries(all).filter(([, mode]) => mode.recommended)).toEqual([
      ['auto', all.auto],
    ]);
  });
});
