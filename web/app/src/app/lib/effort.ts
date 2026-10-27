import type { ModelInfo, RawEvent } from '@glave/client';
import type { GlzMenuOption } from '@glave/glaze';

/**
 * A level as people read it: `xhigh` is `Extra high`, `none` is `Off`; one providers name that
 * the app does not know is its own word, capitalized.
 */
export function levelName(level: string): string {
  switch (level) {
    case 'max':
      return $localize`:@@effort.max:Max`;
    case 'xhigh':
      return $localize`:@@effort.xhigh:Extra high`;
    case 'high':
      return $localize`:@@effort.high:High`;
    case 'medium':
      return $localize`:@@effort.medium:Medium`;
    case 'low':
      return $localize`:@@effort.low:Low`;
    case 'minimal':
      return $localize`:@@effort.minimal:Minimal`;
    case 'none':
      return $localize`:@@effort.none:Off`;
    default:
      return `${level.charAt(0).toUpperCase()}${level.slice(1)}`;
  }
}

/**
 * The picker's choice that asks for no level: Default, the model's own way, as in a thread that
 * never chose. No level is empty.
 */
export const UNASKED = '';

/**
 * What the picker offers for `model`: Default first, named by the level the server says it is
 * where it says one, then the levels it offers, as its provider lists them; nothing when it
 * offers no choice.
 */
export function effortOptions(model: ModelInfo | undefined): readonly GlzMenuOption[] {
  const reasoning = model?.reasoning ?? null;
  const efforts = reasoning?.efforts ?? [];
  if (efforts.length === 0) {
    return [];
  }
  const fallback = reasoning?.default ?? null;
  const named = fallback === null ? {} : { hint: levelName(fallback), short: levelName(fallback) };
  const unasked = { value: UNASKED, label: $localize`:@@effort.default:Default`, ...named };
  return [unasked, ...efforts.map((level) => ({ value: level, label: levelName(level) }))];
}

/** The thread's own choice of level: the latest `effort.chosen`'s, or `null` for Default. */
export function chosenEffort(events: readonly RawEvent[]): string | null {
  const chosen = events.filter((event) => event.type === 'effort.chosen').at(-1);
  const effort = chosen?.data['effort'];
  return typeof effort === 'string' && effort !== '' ? effort : null;
}

/**
 * The level to send with the next turn, when the person's pick changes the thread's choice: a
 * level `model` offers, or `null` for Default, the model's own way again. Else `undefined`,
 * which leaves the thread's choice as it is.
 */
export function effortToSend(
  picked: string | null,
  chosen: string | null,
  model: ModelInfo | undefined,
): string | null | undefined {
  if (picked === UNASKED) {
    return chosen === null ? undefined : null;
  }
  const offered = model?.reasoning?.efforts ?? [];
  return picked !== null && picked !== chosen && offered.includes(picked) ? picked : undefined;
}
