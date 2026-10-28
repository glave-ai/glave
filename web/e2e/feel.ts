import { type Page, expect, test } from '@playwright/test';

/**
 * How the app must feel, the budgets `feel.e2e.ts` holds it to, set in this one place, each
 * measured as web-vitals measures it from the browser's own entries (`watchFeel`). Layout shift
 * comes out the same on every run, so its budget is strict: nothing moves. The times are the
 * thresholds web-vitals calls good, several times what the tests measure on a busy laptop, so a
 * busy runner stays under them and a main thread held up, or a page held back, goes over.
 */
export const FEEL = {
  /** Cumulative Layout Shift, unitless: the worst burst of shifts the person did not cause. */
  shift: 0,
  /** Interaction to Next Paint, in ms: the slowest click, from the press to the next frame. */
  interaction: 200,
  /** First Contentful Paint, in ms from the start of the load. */
  firstPaint: 1_800,
  /** Largest Contentful Paint, in ms from the start of the load. */
  largestPaint: 2_500,
} as const;

/** One layout shift, as a `layout-shift` entry reports it. */
interface Shift {
  readonly value: number;
  readonly startTime: number;
  readonly hadRecentInput: boolean;
  /** What moved, each as its element and its box before and after, in CSS pixels. */
  readonly moved: readonly string[];
}

/** One event of an interaction, as an `event` entry reports it. */
interface Timing {
  readonly interactionId: number;
  readonly duration: number;
}

/** What the page's observers have collected since it loaded, or since it was last told to forget. */
interface Seen {
  readonly shifts: readonly Shift[];
  readonly timings: readonly Timing[];
  readonly inputs: number;
  readonly firstPaint: number | null;
  readonly largestPaint: number | null;
}

/** The window of a page `watchFeel` watches: what it has seen, and a way to forget it. */
type Watched = Window & { feel?: { readonly seen: () => Seen; readonly forget: () => void } };

/** What the person felt on a page, measured as web-vitals measures it. */
export interface Felt {
  /** Cumulative Layout Shift. */
  readonly shift: number;
  /** What moved in the shifts it counts, so a failure says what to look at. */
  readonly moved: readonly string[];
  /** Interaction to Next Paint, in ms; 0 with no click slower than a frame. */
  readonly interaction: number;
  /** How many first inputs the browser reported: one once the person clicked, else none. */
  readonly inputs: number;
  /** First Contentful Paint, in ms; null before it, or once forgotten. */
  readonly firstPaint: number | null;
  /** Largest Contentful Paint, in ms; null before it, or once forgotten. */
  readonly largestPaint: number | null;
}

/**
 * Watches every page `page` loads from now on, before any script of its own runs, with the
 * observers web-vitals uses, buffered so nothing before them is missed: layout shifts, events
 * slower than a frame, the first input, and the paints.
 */
export async function watchFeel(page: Page): Promise<void> {
  await page.addInitScript(() => {
    type Entry = PerformanceEntry & Readonly<Record<string, unknown>>;
    interface Source {
      readonly node: Node | null;
      readonly previousRect: DOMRectReadOnly;
      readonly currentRect: DOMRectReadOnly;
    }
    const box = (rect: DOMRectReadOnly): string =>
      [rect.x, rect.y, rect.width, rect.height].map(Math.round).join(',');
    const described = ({ node, previousRect, currentRect }: Source): string => {
      const element = node instanceof Element ? node : node?.parentElement;
      const id = element?.getAttribute('data-testid');
      const name = `${element?.localName ?? 'text'}${id === null || id === undefined ? '' : `[${id}]`}`;
      return `${name} ${box(previousRect)} → ${box(currentRect)}`;
    };
    let shifts: Shift[] = [];
    let timings: Timing[] = [];
    let inputs = 0;
    let firstPaint: number | null = null;
    let largestPaint: number | null = null;
    const kept: Readonly<Record<string, (entry: Entry) => void>> = {
      'layout-shift': (entry) => {
        shifts.push({
          value: Number(entry['value']),
          startTime: entry.startTime,
          hadRecentInput: entry['hadRecentInput'] === true,
          moved: (entry['sources'] as readonly Source[]).map(described),
        });
      },
      event: (entry) => {
        timings.push({ interactionId: Number(entry['interactionId']), duration: entry.duration });
      },
      'first-input': () => {
        inputs += 1;
      },
      paint: (entry) => {
        if (entry.name === 'first-contentful-paint') {
          firstPaint = entry.startTime;
        }
      },
      'largest-contentful-paint': (entry) => {
        largestPaint = entry.startTime;
      },
    };
    const observers = Object.entries(kept).map(([type, keep]) => {
      const take = (entries: readonly PerformanceEntry[]): void => {
        for (const entry of entries) {
          keep(entry as Entry);
        }
      };
      const observer = new PerformanceObserver((list) => {
        take(list.getEntries());
      });
      // Events slower than a frame, the least the browser reports, as web-vitals asks.
      const slower = type === 'event' ? { durationThreshold: 16 } : {};
      observer.observe({ type, buffered: true, ...slower });
      return { observer, take };
    });
    // Observers report once the browser is idle: what they still hold is taken as it is read.
    const flush = (): void => {
      for (const { observer, take } of observers) {
        take(observer.takeRecords());
      }
    };
    (window as Watched).feel = {
      seen: () => {
        flush();
        return { shifts: [...shifts], timings: [...timings], inputs, firstPaint, largestPaint };
      },
      forget: () => {
        flush();
        shifts = [];
        timings = [];
        inputs = 0;
        firstPaint = null;
        largestPaint = null;
      },
    };
  });
}

/**
 * Cumulative Layout Shift as web-vitals counts it: shifts within half a second of the person's
 * input left out, the rest in bursts (each shift within a second of the one before, five
 * seconds at most), the worst burst's sum.
 */
function cumulative(shifts: readonly Shift[]): number {
  let worst = 0;
  let burst = 0;
  let first = -Infinity;
  let last = -Infinity;
  for (const shift of shifts.filter((each) => !each.hadRecentInput)) {
    if (shift.startTime - last < 1_000 && shift.startTime - first < 5_000) {
      burst += shift.value;
    } else {
      burst = shift.value;
      first = shift.startTime;
    }
    last = shift.startTime;
    worst = Math.max(worst, burst);
  }
  return worst;
}

/**
 * Interaction to Next Paint as web-vitals counts it for fewer than 50 interactions: the slowest
 * interaction, each as long as its slowest event.
 */
function slowest(timings: readonly Timing[]): number {
  const interactions = new Map<number, number>();
  for (const timing of timings.filter((each) => each.interactionId > 0)) {
    const before = interactions.get(timing.interactionId) ?? 0;
    interactions.set(timing.interactionId, Math.max(timing.duration, before));
  }
  return Math.max(0, ...interactions.values());
}

/**
 * Asks the watch on `page` to `act` (what it has seen, or to forget it) after its next frame, so
 * what that frame paints is in.
 */
async function afterFrame(page: Page, act: 'seen' | 'forget'): Promise<Seen | undefined> {
  return page.evaluate(
    (asked) =>
      new Promise<Seen | undefined>((resolve) => {
        requestAnimationFrame(() => {
          setTimeout(() => {
            const feel = (window as Watched).feel;
            if (asked === 'forget') {
              feel?.forget();
            }
            resolve(feel?.seen());
          }, 0);
        });
      }),
    act,
  );
}

/** Forgets what `page` has felt so far, so what is measured next is what comes after. */
export async function forgetFelt(page: Page): Promise<void> {
  await afterFrame(page, 'forget');
}

/**
 * What the person has felt on `page` since it loaded or last forgot, noted on the test's report;
 * fails when `watchFeel` did not watch it.
 */
export async function feltOn(page: Page): Promise<Felt> {
  const seen = await afterFrame(page, 'seen');
  if (seen === undefined) {
    throw new Error('the page was not watched: call watchFeel before it loads');
  }
  const felt: Felt = {
    shift: cumulative(seen.shifts),
    moved: seen.shifts.filter((each) => !each.hadRecentInput).flatMap((each) => each.moved),
    interaction: slowest(seen.timings),
    inputs: seen.inputs,
    firstPaint: seen.firstPaint,
    largestPaint: seen.largestPaint,
  };
  test.info().annotations.push({ type: 'feel', description: JSON.stringify(felt) });
  return felt;
}

/** Fails unless `felt` shows its content within `FEEL`'s budgets. */
export function expectShownSoon(felt: Felt): void {
  expect(felt.firstPaint, 'first content').not.toBeNull();
  expect(felt.largestPaint, 'largest content').not.toBeNull();
  expect(felt.firstPaint ?? Infinity, 'first content').toBeLessThanOrEqual(FEEL.firstPaint);
  expect(felt.largestPaint ?? Infinity, 'largest content').toBeLessThanOrEqual(FEEL.largestPaint);
}

/** Fails unless every click `felt` measured painted its answer within `FEEL`'s budget. */
export function expectAnsweredAtOnce(felt: Felt): void {
  expect(felt.inputs, 'the browser saw the clicks').toBe(1);
  expect(felt.interaction, 'the slowest click until the next paint').toBeLessThanOrEqual(
    FEEL.interaction,
  );
}

/** Fails unless nothing `felt` measured moved beyond `FEEL`'s budget. */
export function expectStill(felt: Felt): void {
  expect(felt.shift, `layout shift, of ${felt.moved.join('; ')}`).toBeLessThanOrEqual(FEEL.shift);
}
