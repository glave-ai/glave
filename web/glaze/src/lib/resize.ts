/** `width` kept between `min` and `max`, in whole pixels. */
export function clampWidth(width: number, min: number, max: number): number {
  return Math.round(Math.min(Math.max(width, min), Math.max(min, max)));
}

/** Which side of the page a resizable panel sits on: the inline start or the inline end. */
export type GlzPanelSide = 'start' | 'end';

/** The text's direction, which decides where the inline start is. */
export type GlzDirection = 'ltr' | 'rtl';

/** Where a dragged edge leaves its panel: open at a width, or collapsed. */
export type GlzSplit = { readonly open: true; readonly width: number } | { readonly open: false };

/** Whether a panel on `side` sits on the left of the page in `direction`. */
function onLeft(side: GlzPanelSide, direction: GlzDirection): boolean {
  return (side === 'start') === (direction === 'ltr');
}

/**
 * Where a panel ends up after its edge moved `delta` pixels along x, from a drag that began
 * with it `start` pixels wide (0 when it began collapsed). The panel widens as its edge moves
 * away from its side of the page. Dragged below half its minimum it collapses; dragged back
 * past that point it opens again, at least `min` wide.
 */
export function draggedSplit(
  start: number,
  delta: number,
  side: GlzPanelSide,
  direction: GlzDirection,
  min: number,
  max: number,
): GlzSplit {
  const raw = start + (onLeft(side, direction) ? delta : -delta);
  return raw < min / 2 ? { open: false } : { open: true, width: clampWidth(raw, min, max) };
}

/** The arrow keys that widen and narrow a panel: the one pointing away from its side widens. */
export function splitterKeys(
  side: GlzPanelSide,
  direction: GlzDirection,
): { readonly grow: string; readonly shrink: string } {
  return onLeft(side, direction)
    ? { grow: 'ArrowRight', shrink: 'ArrowLeft' }
    : { grow: 'ArrowLeft', shrink: 'ArrowRight' };
}
