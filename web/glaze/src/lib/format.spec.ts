import { formatBytes, formatDuration, formatMoney, formatTokens, initials } from './format';

describe('format', () => {
  it('shows money exactly enough', () => {
    expect(formatMoney({ micros: 4200, currency: 'USD' }, 'en-US')).toBe('$0.0042');
    expect(formatMoney({ micros: 1_250_000, currency: 'USD' }, 'en-US')).toBe('$1.25');
    expect(formatMoney({ micros: 0, currency: 'EUR' }, 'en-US')).toBe('€0.00');
  });

  it('shortens token counts', () => {
    expect(formatTokens(950, 'en-US')).toBe('950');
    expect(formatTokens(12_345, 'en-US')).toBe('12.3K');
    expect(formatTokens(4_100_000, 'en-US')).toBe('4.1M');
  });

  it('shortens durations', () => {
    expect(formatDuration(850)).toBe('850 ms');
    expect(formatDuration(12_000)).toBe('12 s');
    expect(formatDuration(200_000)).toBe('3 min 20 s');
    expect(formatDuration(180_000)).toBe('3 min');
    expect(formatDuration(7_500_000)).toBe('2 h 5 min');
    expect(formatDuration(7_200_000)).toBe('2 h');
  });

  it('takes initials from the first and last words', () => {
    expect(initials('Ada Lovelace')).toBe('AL');
    expect(initials(' hope ')).toBe('H');
    expect(initials('Grace Brewster Hopper')).toBe('GH');
    expect(initials('')).toBe('');
  });

  it('shortens file sizes', () => {
    expect(formatBytes(512)).toBe('512 B');
    expect(formatBytes(12_400)).toBe('12 KB');
    expect(formatBytes(3_565_158)).toBe('3.4 MB');
  });
});
