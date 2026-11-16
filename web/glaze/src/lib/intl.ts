/** How many formatters of each kind are kept: more locales and options than a page uses. */
const KEPT = 32;

const numbers = new Map<string, Intl.NumberFormat>();
const lists = new Map<string, Intl.ListFormat>();
const plurals = new Map<string, Intl.PluralRules>();
const dates = new Map<string, Intl.DateTimeFormat>();
const relatives = new Map<string, Intl.RelativeTimeFormat>();

/**
 * The formatter for `locale` (the browser's when absent) and `options`, made once and kept in
 * `kept`: making one costs far more than formatting with it, and a thread writes its turns'
 * facts again as each word of a reply streams in. Past `KEPT`, the oldest goes.
 */
function made<T>(
  kept: Map<string, T>,
  locale: string | undefined,
  options: object,
  make: () => T,
): T {
  const key = JSON.stringify([locale ?? null, options]);
  const found = kept.get(key);
  if (found !== undefined) {
    return found;
  }
  const oldest = kept.keys().next();
  if (kept.size >= KEPT && oldest.done !== true) {
    kept.delete(oldest.value);
  }
  const fresh = make();
  kept.set(key, fresh);
  return fresh;
}

/** `Intl.NumberFormat` for `locale` and `options`, made once. */
export function numberFormat(
  locale?: string,
  options: Intl.NumberFormatOptions = {},
): Intl.NumberFormat {
  return made(numbers, locale, options, () => new Intl.NumberFormat(locale, options));
}

/** `Intl.ListFormat` for `locale` and `options`, made once. */
export function listFormat(locale?: string, options: Intl.ListFormatOptions = {}): Intl.ListFormat {
  return made(lists, locale, options, () => new Intl.ListFormat(locale, options));
}

/** `Intl.PluralRules` for `locale`, made once. */
export function pluralRules(locale?: string): Intl.PluralRules {
  return made(plurals, locale, {}, () => new Intl.PluralRules(locale));
}

/**
 * `Intl.DateTimeFormat` for `locale` and `options`, made once. A `timeZone` the platform does
 * not know throws, as the constructor does.
 */
export function dateTimeFormat(
  locale?: string,
  options: Intl.DateTimeFormatOptions = {},
): Intl.DateTimeFormat {
  return made(dates, locale, options, () => new Intl.DateTimeFormat(locale, options));
}

/** `Intl.RelativeTimeFormat` for `locale` and `options`, made once. */
export function relativeTimeFormat(
  locale?: string,
  options: Intl.RelativeTimeFormatOptions = {},
): Intl.RelativeTimeFormat {
  return made(relatives, locale, options, () => new Intl.RelativeTimeFormat(locale, options));
}
