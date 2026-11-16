import type { ScheduleInfo } from '@glave/client';

/** A schedule's run time as its line shows it, in the reader's language and clock: "Tue 8:00 AM". */
export function runAt(at: string, locale: string): string {
  const format = new Intl.DateTimeFormat(locale, {
    weekday: 'short',
    hour: 'numeric',
    minute: '2-digit',
  });
  return format.format(Date.parse(at));
}

/** When `schedule` runs next, or that it is paused; `null` once it runs no more. */
export function nextOf(schedule: ScheduleInfo, locale: string): string | null {
  if (schedule.paused) {
    return $localize`:@@repeating.paused:Paused`;
  }
  const next = schedule.next ?? null;
  if (next === null) {
    return null;
  }
  const time = runAt(next, locale);
  return $localize`:@@repeating.next:Next: ${time}:TIME:`;
}

/** How `schedule`'s latest run went, by its status as the server says it; `null` before one. */
export function lastOf(schedule: ScheduleInfo, locale: string): string | null {
  const last = schedule.last ?? null;
  if (last === null) {
    return null;
  }
  const time = runAt(last.due, locale);
  switch (last.status) {
    case 'running':
      return $localize`:@@repeating.lastRunning:Last: ${time}:TIME:, running`;
    case 'skipped':
      return $localize`:@@repeating.lastSkipped:Last: ${time}:TIME:, skipped while the one before was still going`;
    case 'ended':
      return last.outcome === 'completed'
        ? $localize`:@@repeating.lastDone:Last: ${time}:TIME:, done`
        : $localize`:@@repeating.lastUnfinished:Last: ${time}:TIME:, didn't finish`;
    case 'started':
      return $localize`:@@repeating.last:Last: ${time}:TIME:`;
  }
}

/**
 * `schedule` in a line: when it repeats, in the server's words, then its next run, or that it
 * is paused, and how its last run went.
 */
export function repeatLine(schedule: ScheduleInfo, locale: string): string {
  const parts = [schedule.repeats, nextOf(schedule, locale), lastOf(schedule, locale)];
  return parts.filter((part): part is string => part !== null).join(' · ');
}
