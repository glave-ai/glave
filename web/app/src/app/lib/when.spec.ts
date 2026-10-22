import { type Reader, readerTimes, runningFor } from './when';

const READER: Reader = { locale: 'en-US', timeZone: 'UTC' };
const NOW = Date.parse('2026-10-14T12:00:00.000Z');

describe('when', () => {
  it('says how long ago, short on a row and in words for a screen reader', () => {
    const times = readerTimes(NOW, READER);
    const shown = (at: string): readonly string[] => {
      const { short, long } = times.since(at);
      return [short, long];
    };
    expect(shown('2026-10-14T11:59:30.000Z')).toEqual(['now', 'now']);
    expect(shown('2026-10-14T11:55:00.000Z')).toEqual(['5m', '5 minutes ago']);
    expect(shown('2026-10-14T09:00:00.000Z')).toEqual(['3h', '3 hours ago']);
    expect(shown('2026-10-12T12:00:00.000Z')).toEqual(['2d', '2 days ago']);
    expect(shown('2026-10-01T08:00:00.000Z')).toEqual(['Oct 1', 'October 1']);
    expect(shown('2025-12-30T08:00:00.000Z')).toEqual(['Dec 30, 2025', 'December 30, 2025']);
    expect(times.since('2026-10-14T11:55:00.000Z').at).toBe('2026-10-14T11:55:00.000Z');
  });

  it('takes a time ahead of this clock as now, and one it cannot read as none', () => {
    const times = readerTimes(NOW, READER);
    expect(times.since('2026-10-14T12:00:40.000Z').short).toBe('now');
    expect(times.since('soon')).toEqual({ at: 'soon', short: '', long: '' });
  });

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
    expect(times.since('2026-10-01T08:00:00.000Z').short).toBe('1 Oct');
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

describe('runningFor', () => {
  const start = '2026-10-11T10:00:00.000Z';
  const at = (offset: number): number => Date.parse(start) + offset;

  it('is how long it has run, in whole seconds, and at least one', () => {
    expect(runningFor(start, at(125_400))).toBe(125_000);
    expect(runningFor(start, at(999))).toBe(1000);
    // A clock behind the server's still reads a second.
    expect(runningFor(start, at(-5000))).toBe(1000);
  });

  it('is unknown until the start is recorded, or when it does not parse', () => {
    expect(runningFor(null, at(0))).toBeNull();
    expect(runningFor(undefined, at(0))).toBeNull();
    expect(runningFor('soon', at(0))).toBeNull();
  });
});
