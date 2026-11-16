/** The reader the server's times are written for: their language and time zone, by default. */
export interface Reader {
  readonly locale?: string;
  readonly timeZone?: string;
}

/**
 * When something happened, as a row shows it, short ("now", "5m", "3h", "2d", "Oct 12"), and as a
 * screen reader says it ("5 minutes ago", "October 12").
 */
export interface Since {
  /** The server's time, for a `<time>`'s `datetime`. */
  readonly at: string;
  readonly short: string;
  readonly long: string;
}

/** The server's times, for one reader at one moment. */
export interface Times {
  /** How long before now `at` was. */
  since(at: string): Since;
  /** The ways of writing the day of `at` a person might search by: its date, its weekday, today. */
  day(at: string): readonly string[];
}

/**
 * How long something that started at `startedAt`, as the server recorded it, has gone on at
 * `now` (milliseconds since the epoch), in whole seconds, as milliseconds: at least a second, so
 * a clock never reads less. `null` when its start is not recorded yet or does not parse.
 */
export function runningFor(startedAt: string | null | undefined, now: number): number | null {
  const start = typeof startedAt === 'string' ? Date.parse(startedAt) : Number.NaN;
  if (Number.isNaN(start)) {
    return null;
  }
  return Math.max(1, Math.floor((now - start) / 1000)) * 1000;
}

const MINUTE = 60_000;
const HOUR = 60 * MINUTE;
const DAY = 24 * HOUR;

/** The units a recent time counts in, each until the next one's size; a week on, the date. */
const UNITS = [
  { unit: 'minute', size: MINUTE, until: HOUR },
  { unit: 'hour', size: HOUR, until: DAY },
  { unit: 'day', size: DAY, until: 7 * DAY },
] as const;

/**
 * The server's times as `reader` reads them at `now` (milliseconds since the epoch): written in
 * their language and counted in their time zone. A time after `now`, as a clock a little ahead
 * of this one writes it, is now.
 */
export function readerTimes(now: number, reader: Reader = {}): Times {
  const { locale, timeZone } = reader;
  const relative = new Intl.RelativeTimeFormat(locale, { numeric: 'auto' });
  const ago = new Intl.RelativeTimeFormat(locale);
  const units = UNITS.map((each) => ({
    ...each,
    short: new Intl.NumberFormat(locale, { style: 'unit', unit: each.unit, unitDisplay: 'narrow' }),
  }));
  const date = (options: Intl.DateTimeFormatOptions): Intl.DateTimeFormat =>
    new Intl.DateTimeFormat(locale, { timeZone, ...options });
  const thisYear = {
    short: date({ month: 'short', day: 'numeric' }),
    long: date({ month: 'long', day: 'numeric' }),
  };
  const otherYear = { short: date({ dateStyle: 'medium' }), long: date({ dateStyle: 'long' }) };
  // In words with its weekday ("oct 12" begins its words, in any order), and in numbers.
  const searched = [
    date({ dateStyle: 'full' }),
    date({ year: 'numeric', month: 'numeric', day: 'numeric' }),
  ];
  const dayOf = calendarDay(timeZone);
  const today = dayOf(now);

  return {
    since(at) {
      const then = Date.parse(at);
      if (Number.isNaN(then)) {
        return { at, short: '', long: '' };
      }
      const gone = Math.max(0, now - then);
      if (gone < MINUTE) {
        const word = relative.format(0, 'second');
        return { at, short: word, long: word };
      }
      const unit = units.find(({ until }) => gone < until);
      if (unit !== undefined) {
        const count = Math.floor(gone / unit.size);
        return { at, short: unit.short.format(count), long: ago.format(-count, unit.unit) };
      }
      const forms = dayOf(then).slice(0, 4) === today.slice(0, 4) ? thisYear : otherYear;
      return { at, short: forms.short.format(then), long: forms.long.format(then) };
    },
    day(at) {
      const then = Date.parse(at);
      if (Number.isNaN(then)) {
        return [];
      }
      const day = dayOf(then);
      const before = Math.round((Date.parse(today) - Date.parse(day)) / DAY);
      const near = before === 0 || before === 1 ? [relative.format(-before, 'day')] : [];
      return [...searched.map((form) => form.format(then)), day, ...near];
    },
  };
}

/** The day a moment falls on in `timeZone`, written `2026-10-12`. */
function calendarDay(timeZone: string | undefined): (at: number) => string {
  const parts = new Intl.DateTimeFormat('en-US', {
    timeZone,
    year: 'numeric',
    month: '2-digit',
    day: '2-digit',
  });
  return (at) => {
    const found = parts.formatToParts(at);
    const part = (type: Intl.DateTimeFormatPartTypes): string =>
      found.find((each) => each.type === type)?.value ?? '';
    return `${part('year')}-${part('month')}-${part('day')}`;
  };
}
