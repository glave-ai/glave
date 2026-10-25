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
 * What happens when no level is chosen: marking the level sent then, or, where none is sent,
 * saying the model reasons as its maker set it.
 */
export function defaultLabel(): string {
  return $localize`:@@effort.default:Default`;
}

/**
 * The levels of reasoning `model` offers, as its provider lists them, the one sent when none
 * is chosen marked: none when it offers no choice.
 */
export function effortOptions(model: ModelInfo | undefined): readonly GlzMenuOption[] {
  const reasoning = model?.reasoning ?? null;
  const efforts = reasoning?.efforts ?? [];
  const fallback = reasoning?.default ?? null;
  const hint = defaultLabel();
  return efforts.map((level) => ({
    value: level,
    label: levelName(level),
    ...(level === fallback ? { hint } : {}),
  }));
}

/**
 * The level the server says `model` is sent when none is chosen, or `null` when it is sent
 * none, so it reasons as its maker set it.
 */
export function defaultEffort(model: ModelInfo | undefined): string | null {
  return model?.reasoning?.default ?? null;
}

/** The thread's own choice of level: the latest `effort.chosen`'s, or `null` for none. */
export function chosenEffort(events: readonly RawEvent[]): string | null {
  const chosen = events.filter((event) => event.type === 'effort.chosen').at(-1);
  const effort = chosen?.data['effort'];
  return typeof effort === 'string' && effort !== '' ? effort : null;
}

/**
 * The level to send with the next turn: the person's pick, when `model` offers it and it is
 * not the thread's choice already; else `null`, which leaves the thread's choice as it is.
 */
export function effortToSend(
  picked: string | null,
  chosen: string | null,
  model: ModelInfo | undefined,
): string | null {
  const offered = model?.reasoning?.efforts ?? [];
  return picked !== null && picked !== chosen && offered.includes(picked) ? picked : null;
}
