import type { GlzDirection } from './resize';

/**
 * Where a popover shows, relative to the element it belongs to: above or below it, or at the
 * start or end of its line, which in right-to-left text are its right and its left.
 */
export type GlzSide = 'above' | 'below' | 'start' | 'end';

/** A box on screen, in CSS pixels. */
export interface GlzBox {
  readonly top: number;
  readonly left: number;
  readonly width: number;
  readonly height: number;
}

/** The viewport's size, in CSS pixels. */
export interface GlzViewport {
  readonly width: number;
  readonly height: number;
}

/**
 * Where a popover goes when it does not fit on its side: the opposite side, or whichever side
 * has the most room, for a popover that shrinks to the room it has (the picker's).
 */
export type GlzFallback = 'opposite' | 'most-room';

/** The gap between a popover and its anchor, in CSS pixels. */
const GAP = 4;

/**
 * Where a `most-room` popover is measured, the room on the side it takes (a height above or
 * below, a width at the start or end), for its styles to cap it to: where anchor positioning
 * places it, `100%` of its area is that room.
 */
const ROOM = '--glz-popover-room';

const OPPOSITE: Readonly<Record<GlzSide, GlzSide>> = {
  above: 'below',
  below: 'above',
  start: 'end',
  end: 'start',
};

/** Each side as measured on the screen, where `start` is the left: right to left, they swap. */
export function screenSide(side: GlzSide, direction: GlzDirection): GlzSide {
  return direction === 'rtl' && (side === 'start' || side === 'end') ? OPPOSITE[side] : side;
}

/** Where `panel` goes on `side` of `anchor`, `gap` away, without checking it fits. */
function placed(anchor: GlzBox, panel: GlzViewport, side: GlzSide, gap: number): GlzBox {
  const centreLeft = anchor.left + (anchor.width - panel.width) / 2;
  const centreTop = anchor.top + (anchor.height - panel.height) / 2;
  const positions: Readonly<Record<GlzSide, { top: number; left: number }>> = {
    above: { top: anchor.top - panel.height - gap, left: centreLeft },
    below: { top: anchor.top + anchor.height + gap, left: centreLeft },
    start: { top: centreTop, left: anchor.left - panel.width - gap },
    end: { top: centreTop, left: anchor.left + anchor.width + gap },
  };
  return { ...positions[side], width: panel.width, height: panel.height };
}

function fits(box: GlzBox, viewport: GlzViewport): boolean {
  return (
    box.top >= 0 &&
    box.left >= 0 &&
    box.top + box.height <= viewport.height &&
    box.left + box.width <= viewport.width
  );
}

function clamp(value: number, size: number, limit: number): number {
  return Math.max(0, Math.min(value, limit - size));
}

/** The room beside `anchor` on `side`, `gap` away, inside the viewport. */
function room(anchor: GlzBox, viewport: GlzViewport, side: GlzSide, gap: number): number {
  const rooms: Readonly<Record<GlzSide, number>> = {
    above: anchor.top,
    below: viewport.height - anchor.top - anchor.height,
    start: anchor.left,
    end: viewport.width - anchor.left - anchor.width,
  };
  return Math.max(0, rooms[side] - gap);
}

/**
 * The side a popover of `panel`'s size takes beside `anchor`: `side` if it fits, else the
 * opposite side, or with `most-room` whichever of the two has more room.
 */
function sideTaken(
  anchor: GlzBox,
  panel: GlzViewport,
  viewport: GlzViewport,
  side: GlzSide,
  fallback: GlzFallback,
  gap: number,
): GlzSide {
  const other = OPPOSITE[side];
  if (fits(placed(anchor, panel, side, gap), viewport)) {
    return side;
  }
  if (fallback === 'opposite') {
    return other;
  }
  return room(anchor, viewport, other, gap) > room(anchor, viewport, side, gap) ? other : side;
}

/**
 * Where to put a popover of `panel`'s size beside `anchor`: on `side` if it fits, else on the
 * opposite side (with `most-room`, the side with more room), kept inside the viewport either
 * way, `start` the screen's left (`screenSide`). For browsers without CSS anchor positioning,
 * which is not yet Baseline.
 */
export function placePopover(
  anchor: GlzBox,
  panel: GlzViewport,
  viewport: GlzViewport,
  side: GlzSide,
  fallback: GlzFallback = 'opposite',
  gap = GAP,
): { readonly top: number; readonly left: number } {
  const box = placed(anchor, panel, sideTaken(anchor, panel, viewport, side, fallback, gap), gap);
  return {
    top: clamp(box.top, panel.height, viewport.height),
    left: clamp(box.left, panel.width, viewport.width),
  };
}

const POSITION_AREA: Readonly<Record<GlzSide, string>> = {
  above: 'block-start',
  below: 'block-end',
  start: 'inline-start',
  end: 'inline-end',
};

/** The `position-try-order` that tries the side with the most room first. */
const MOST_ROOM: Readonly<Record<GlzSide, string>> = {
  above: 'most-height',
  below: 'most-height',
  start: 'most-width',
  end: 'most-width',
};

/** Whether this browser has CSS anchor positioning. */
export function hasAnchorPositioning(view: Window & typeof globalThis): boolean {
  return 'CSS' in view && view.CSS.supports('anchor-name', '--glz');
}

/**
 * Positions the open popover `panel` on `side` of `anchor`: with CSS anchor positioning where
 * the browser has it (flipping to the other side when there is no room), else by measuring.
 * With `most-room`, a popover with no room on `side` takes the side with the most, and when
 * measured, its room there is set as `--glz-popover-room` for its styles to cap it to. `name`
 * is a unique dashed ident for the anchor. Call after `showPopover()`, and again when the page
 * scrolls or resizes in browsers without anchor positioning.
 */
export function positionPopover(
  anchor: HTMLElement,
  panel: HTMLElement,
  side: GlzSide,
  name: string,
  fallback: GlzFallback = 'opposite',
): void {
  const view = anchor.ownerDocument.defaultView;
  if (view === null) {
    return;
  }
  panel.style.setProperty('margin', '0');
  panel.style.setProperty('position', 'fixed');
  if (hasAnchorPositioning(view)) {
    anchor.style.setProperty('anchor-name', name);
    panel.style.setProperty('position-anchor', name);
    panel.style.setProperty('position-area', POSITION_AREA[side]);
    panel.style.setProperty('position-try-fallbacks', 'flip-block, flip-inline');
    panel.style.setProperty(
      'position-try-order',
      fallback === 'most-room' ? MOST_ROOM[side] : 'normal',
    );
    panel.style.setProperty('inset', 'auto');
    return;
  }
  const rect = anchor.getBoundingClientRect();
  const box = { top: rect.top, left: rect.left, width: rect.width, height: rect.height };
  const viewport = { width: view.innerWidth, height: view.innerHeight };
  const direction = view.getComputedStyle(anchor).direction === 'rtl' ? 'rtl' : 'ltr';
  const onScreen = screenSide(side, direction);
  if (fallback === 'most-room') {
    panel.style.removeProperty(ROOM);
    const taken = sideTaken(box, size(panel), viewport, onScreen, fallback, GAP);
    panel.style.setProperty(ROOM, `${String(room(box, viewport, taken, GAP))}px`);
  }
  const at = placePopover(box, size(panel), viewport, onScreen, fallback);
  panel.style.setProperty('inset', `${String(at.top)}px auto auto ${String(at.left)}px`);
}

/** The size `element` is laid out at. */
function size(element: HTMLElement): GlzViewport {
  const rect = element.getBoundingClientRect();
  return { width: rect.width, height: rect.height };
}
