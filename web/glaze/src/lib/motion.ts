/** The durations of the motion tokens (styles/tokens.css) that what moves by script reads. */
export type GlzDuration = 'quick' | 'standard' | 'slow';

/**
 * How long `duration` lasts on `document`'s page now, in milliseconds, as its token says: 0 when
 * the person asks for less motion, and where the tokens are not on the page.
 */
export function durationOf(document: Document, duration: GlzDuration): number {
  const view = document.defaultView;
  if (view === null) {
    return 0;
  }
  const value = view
    .getComputedStyle(document.documentElement)
    .getPropertyValue(`--glz-duration-${duration}`)
    .trim();
  const number = Number.parseFloat(value);
  if (!Number.isFinite(number) || number <= 0) {
    return 0;
  }
  return value.endsWith('ms') ? number : value.endsWith('s') ? number * 1000 : 0;
}

/**
 * Counts from `from` up to `to` over the slow duration, eased out as the motion tokens say,
 * handing `step` each whole number on the way and `to` last. At once, `to` alone, when the
 * person asks for less motion or the browser cannot time it (the Web Animations API). Returns
 * what stops it where it is.
 */
export function countUp(
  document: Document,
  from: number,
  to: number,
  step: (value: number) => void,
): () => void {
  const view = document.defaultView;
  const duration = durationOf(document, 'slow');
  if (view === null || duration === 0 || !('KeyframeEffect' in view) || !('Animation' in view)) {
    step(to);
    return () => undefined;
  }
  const easing =
    view.getComputedStyle(document.documentElement).getPropertyValue('--glz-ease-out').trim() ||
    'linear';
  // Moves nothing: the animation is only the clock, its progress eased by the token's curve.
  const effect = new view.KeyframeEffect(null, null, { duration, easing, fill: 'forwards' });
  const animation = new view.Animation(effect, document.timeline);
  let frame = 0;
  const tick = (): void => {
    if (animation.playState === 'finished') {
      step(to);
      return;
    }
    const progress = effect.getComputedTiming().progress ?? 0;
    step(Math.round(from + (to - from) * progress));
    frame = view.requestAnimationFrame(tick);
  };
  animation.play();
  frame = view.requestAnimationFrame(tick);
  return () => {
    view.cancelAnimationFrame(frame);
    animation.cancel();
  };
}
