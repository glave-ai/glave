import { type Reader, readerTimes } from './when';

const READER: Reader = { locale: 'en-US', timeZone: 'UTC' };
const NOW = Date.parse('2026-10-14T12:00:00.000Z');

describe('when', () => {
  it('writes a day each way it might be searched by, today and yesterday too', () => {
    const times = readerTimes(NOW, READER);
    expect(times.day('2026-10-14T01:00:00.000Z')).toEqual([
      'Wednesday, October 14, 2026',
      '10/14/2026',
      '2026-10-14',
      'today',
    ]);
    expect(times.day('2026-10-13T23:00:00.000Z')).toContain('yesterday');
    expect(times.day('2026-10-12T23:00:00.000Z')).not.toContain('yesterday');
    expect(times.day('2026-10-12T23:00:00.000Z')).not.toContain('today');
  });

  it('writes the reader’s own language', () => {
    const times = readerTimes(NOW, { locale: 'en-GB', timeZone: 'UTC' });
    expect(times.day('2026-10-01T08:00:00.000Z')).toContain('01/10/2026');
  });

  it('counts days in the reader’s time zone', () => {
    // 01:00 in UTC is still the day before in Los Angeles.
    const times = readerTimes(NOW, { locale: 'en-US', timeZone: 'America/Los_Angeles' });
    expect(times.day('2026-10-14T01:00:00.000Z')).toContain('2026-10-13');
    expect(times.day('2026-10-14T01:00:00.000Z')).toContain('yesterday');
  });

  it('has no day for a time it cannot read', () => {
    expect(readerTimes(NOW, READER).day('soon')).toEqual([]);
  });
});
