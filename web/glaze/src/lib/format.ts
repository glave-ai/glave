import { listFormat, numberFormat } from './intl';

/** An exact amount of money: `micros` millionths of one unit of `currency` (ISO 4217). */
export interface GlzMoney {
  readonly micros: number;
  readonly currency: string;
}

/**
 * Money as people read it: `$0.0042` for small amounts (two significant digits below a cent),
 * `$1.25` above. `locale` defaults to the browser's.
 */
export function formatMoney(money: GlzMoney, locale?: string): string {
  const amount = money.micros / 1_000_000;
  const small = amount !== 0 && Math.abs(amount) < 0.01;
  return numberFormat(locale, {
    style: 'currency',
    currency: money.currency,
    ...(small
      ? { maximumSignificantDigits: 2 }
      : { minimumFractionDigits: 2, maximumFractionDigits: 2 }),
  }).format(amount);
}

/** A token count, short: `950`, `12.3k`, `4.1M`. */
export function formatTokens(tokens: number, locale?: string): string {
  return numberFormat(locale, {
    notation: 'compact',
    maximumFractionDigits: 1,
  }).format(tokens);
}

/**
 * A duration in milliseconds, short, as a clock reads in a line of facts, in `locale`'s units
 * (the browser's by default): `850ms`, `12s`, `4m 02s`, `2h 05m`.
 */
export function formatDuration(ms: number, locale?: string): string {
  const unit = (value: number, name: string, digits = 1): string =>
    numberFormat(locale, {
      style: 'unit',
      unit: name,
      unitDisplay: 'narrow',
      minimumIntegerDigits: digits,
    }).format(value);
  if (ms < 1000) {
    return unit(Math.round(ms), 'millisecond');
  }
  const seconds = Math.round(ms / 1000);
  if (seconds < 60) {
    return unit(seconds, 'second');
  }
  const minutes = Math.floor(seconds / 60);
  const both = listFormat(locale, { style: 'narrow', type: 'unit' });
  if (minutes < 60) {
    return both.format([unit(minutes, 'minute'), unit(seconds % 60, 'second', 2)]);
  }
  return both.format([unit(Math.floor(minutes / 60), 'hour'), unit(minutes % 60, 'minute', 2)]);
}

/** The initials of a name, at most two: `Ada Lovelace` is `AL`, `hope` is `H`. */
export function initials(name: string): string {
  const words = name
    .trim()
    .split(/\s+/u)
    .filter((word) => word !== '');
  const first = words[0]?.[0] ?? '';
  const last = words.length > 1 ? (words[words.length - 1]?.[0] ?? '') : '';
  return `${first}${last}`.toUpperCase();
}
