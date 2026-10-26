import { listFormat, numberFormat, pluralRules } from './intl';

describe('intl', () => {
  it('makes each formatter once for its locale and options', () => {
    const euros = numberFormat('de', { style: 'currency', currency: 'EUR' });
    expect(numberFormat('de', { style: 'currency', currency: 'EUR' })).toBe(euros);
    expect(numberFormat('fr', { style: 'currency', currency: 'EUR' })).not.toBe(euros);
    expect(numberFormat('de', { style: 'currency', currency: 'USD' })).not.toBe(euros);
    expect(euros.format(1.5)).toBe(
      new Intl.NumberFormat('de', { style: 'currency', currency: 'EUR' }).format(1.5),
    );
    expect(listFormat('en', { type: 'unit' })).toBe(listFormat('en', { type: 'unit' }));
    expect(listFormat('en', { type: 'unit' }).format(['4m', '02s'])).toBe('4m, 02s');
    expect(pluralRules('ar')).toBe(pluralRules('ar'));
    expect(pluralRules('ar').select(3)).toBe('few');
  });

  it('keeps a few, the oldest going first', () => {
    const first = numberFormat('en-GB', { maximumFractionDigits: 0 });
    const kept = numberFormat('en-GB', { maximumFractionDigits: 1 });
    for (const minimumIntegerDigits of [1, 2]) {
      for (let maximumFractionDigits = 0; maximumFractionDigits <= 15; maximumFractionDigits += 1) {
        numberFormat('en-GB', { minimumIntegerDigits, maximumFractionDigits });
      }
    }
    // 32 more were made after the first: it went, and the one made after it with it.
    expect(numberFormat('en-GB', { maximumFractionDigits: 0 })).not.toBe(first);
    expect(numberFormat('en-GB', { maximumFractionDigits: 1 })).not.toBe(kept);
  });
});
