import type { ScheduleInfo } from '@glave/client';
import { dateTimeFormat, relativeTimeFormat } from '@glave/glaze';

import { calendarDay } from './when';

/** Who a schedule's times are written for: their language and their own time zone. */
export interface ScheduleReader {
  readonly locale: string;
  /** The reader's own time zone, by its IANA name: a time in another names its zone. */
  readonly timeZone: string;
}

const DAY = 86_400_000;

/**
 * The zone a schedule's times are written in: its own, which the server reads it in, else the
 * reader's when the platform does not know it.
 */
function zoneFor(zone: string, reader: ScheduleReader): string {
  try {
    dateTimeFormat(reader.locale, { timeZone: zone });
    return zone;
  } catch {
    return reader.timeZone;
  }
}

/** A time of day at `at` in `zone`, naming the zone when it is not the reader's: "8:00 AM GMT". */
function timeOf(at: number, zone: string, reader: ScheduleReader): string {
  const named = zone === reader.timeZone ? {} : { timeZoneName: 'short' as const };
  return dateTimeFormat(reader.locale, {
    hour: 'numeric',
    minute: '2-digit',
    timeZone: zone,
    ...named,
  }).format(at);
}

/**
 * A schedule's run at `at`, as its line says it to `reader` at `now` (milliseconds since the
 * epoch), in the schedule's `zone`: "tomorrow, 8:00 AM"; "Saturday, 7:00 PM" within a week;
 * else its date, "Nov 30, 8:00 AM". `null` for a time that does not parse.
 */
export function runAt(
  at: string,
  zone: string,
  reader: ScheduleReader,
  now: number,
): string | null {
  const then = Date.parse(at);
  if (Number.isNaN(then)) {
    return null;
  }
  const clocks = zoneFor(zone, reader);
  const dayOf = calendarDay(clocks);
  const [thatDay, today] = [dayOf(then), dayOf(now)];
  const days = Math.round((Date.parse(thatDay) - Date.parse(today)) / DAY);
  const date: Intl.DateTimeFormatOptions =
    Math.abs(days) < 7
      ? { weekday: 'long' }
      : {
          month: 'short',
          day: 'numeric',
          ...(thatDay.slice(0, 4) === today.slice(0, 4) ? {} : { year: 'numeric' }),
        };
  const day =
    Math.abs(days) <= 1
      ? relativeTimeFormat(reader.locale, { numeric: 'auto' }).format(days, 'day')
      : dateTimeFormat(reader.locale, { timeZone: clocks, ...date }).format(then);
  const time = timeOf(then, clocks, reader);
  return $localize`:@@repeating.dayTime:${day}:DAY:, ${time}:TIME:`;
}

/**
 * The date and time a schedule's run was due, as the run's heading says it, in the schedule's
 * `zone`: "Thursday, November 12 · 8:00 AM". `null` for a time that does not parse.
 */
export function runDate(at: string, zone: string, reader: ScheduleReader): string | null {
  const then = Date.parse(at);
  if (Number.isNaN(then)) {
    return null;
  }
  const clocks = zoneFor(zone, reader);
  const date = dateTimeFormat(reader.locale, {
    weekday: 'long',
    month: 'long',
    day: 'numeric',
    timeZone: clocks,
  }).format(then);
  const time = timeOf(then, clocks, reader);
  return $localize`:@@repeating.runDate:${date}:DATE: · ${time}:TIME:`;
}

/** When `schedule` runs next, or that it is paused; `null` once it runs no more. */
export function nextOf(schedule: ScheduleInfo, reader: ScheduleReader, now: number): string | null {
  if (schedule.paused) {
    return $localize`:@@repeating.paused:Paused`;
  }
  const time = runAt(schedule.next ?? '', schedule.zone, reader, now);
  return time === null ? null : $localize`:@@repeating.next:Next: ${time}:TIME:`;
}

/** When `schedule` last ran, and what came of it in the server's words; `null` before a run. */
export function lastOf(schedule: ScheduleInfo, reader: ScheduleReader, now: number): string | null {
  const last = schedule.last ?? null;
  const time = last === null ? null : runAt(last.due, schedule.zone, reader, now);
  if (last === null || time === null) {
    return null;
  }
  const said = last.said;
  return $localize`:@@repeating.last:Last: ${time}:TIME: (${said}:SAID:)`;
}

/** The parts of a line that it has, between dots. */
function line(parts: readonly (string | null | undefined)[]): string {
  return parts.filter((part) => part !== null && part !== undefined && part !== '').join(' · ');
}

/**
 * The line under a schedule's chat's title: when it repeats in the server's words, when it runs
 * next or that it is paused, and what the server says of it beyond that.
 */
export function repeatLine(schedule: ScheduleInfo, reader: ScheduleReader, now: number): string {
  return line([schedule.repeats, nextOf(schedule, reader, now), schedule.note]);
}

/**
 * A schedule as the Repeating list says it: when it repeats in the server's words, its next run
 * or that it is paused, its last run and what came of it, and what the server says beyond that.
 */
export function scheduleLine(schedule: ScheduleInfo, reader: ScheduleReader, now: number): string {
  return line([
    schedule.repeats,
    nextOf(schedule, reader, now),
    lastOf(schedule, reader, now),
    schedule.note,
  ]);
}
