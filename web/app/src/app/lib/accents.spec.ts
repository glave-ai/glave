import { brandSwatches, personSwatches } from './accents';

describe('accents', () => {
  it('names each swatch, its value its color', () => {
    for (const swatch of brandSwatches()) {
      expect(swatch.value).toBe(swatch.color);
      expect(swatch.value).toMatch(/^#[0-9a-f]{6}$/u);
      expect(swatch.label).not.toBe('');
    }
  });

  it('offers an organization a person’s swatches and two more', () => {
    const person = personSwatches().map((swatch) => swatch.label);
    const brand = brandSwatches().map((swatch) => swatch.label);
    expect(brand).toEqual(['Blue', 'Lime', ...person]);
  });
});
