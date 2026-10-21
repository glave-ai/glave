/** The reader the server's times are written for: their language and time zone, by default. */
export interface Reader {
  readonly locale?: string;
  readonly timeZone?: string;
}

/** The server's times, for one reader at one moment. */
export interface Times {
  /** The ways of writing the day of `at` a person might search by: its date, its weekday, today. */
  day(at: string): readonly string[];
}

const DAY = 24 * 60 * 60_000;

/**
 * The server's times as `reader` reads them at `now` (milliseconds since the epoch): written in
 * their language and counted in their time zone.
 */
export function readerTimes(now: number, reader: Reader = {}): Times {
  const { locale, timeZone } = reader;
  const relative = new Intl.RelativeTimeFormat(locale, { numeric: 'auto' });
  // In words with its weekday ("oct 12" begins its words, in any order), and in numbers.
  const searched = [
    new Intl.DateTimeFormat(locale, { timeZone, dateStyle: 'full' }),
    new Intl.DateTimeFormat(locale, {
      timeZone,
      year: 'numeric',
      month: 'numeric',
      day: 'numeric',
    }),
  ];
  const dayOf = calendarDay(timeZone);
  const today = dayOf(now);

  return {
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
