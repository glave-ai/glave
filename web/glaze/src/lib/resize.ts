/** `width` kept between `min` and `max`, in whole pixels. */
export function clampWidth(width: number, min: number, max: number): number {
  return Math.round(Math.min(Math.max(width, min), Math.max(min, max)));
}

/**
 * The width of a panel at the end of the page after its start edge moved `delta` pixels
 * along x: in left-to-right text, moving it left (a negative delta) widens the panel; in
 * right-to-left text the panel is on the left, so moving it right widens it.
 */
export function resizedWidth(
  start: number,
  delta: number,
  direction: 'ltr' | 'rtl',
  min: number,
  max: number,
): number {
  return clampWidth(direction === 'rtl' ? start + delta : start - delta, min, max);
}
