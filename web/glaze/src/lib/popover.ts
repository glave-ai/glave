/** Where a popover shows, relative to the element it belongs to. */
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

const OPPOSITE: Readonly<Record<GlzSide, GlzSide>> = {
  above: 'below',
  below: 'above',
  start: 'end',
  end: 'start',
};

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

/**
 * Where to put a popover of `panel`'s size beside `anchor`: on `side` if it fits, else on the
 * opposite side, kept inside the viewport either way. For browsers without CSS anchor
 * positioning, which is not yet Baseline.
 */
export function placePopover(
  anchor: GlzBox,
  panel: GlzViewport,
  viewport: GlzViewport,
  side: GlzSide,
  gap = 4,
): { readonly top: number; readonly left: number } {
  const wanted = placed(anchor, panel, side, gap);
  const box = fits(wanted, viewport) ? wanted : placed(anchor, panel, OPPOSITE[side], gap);
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

/** Whether this browser has CSS anchor positioning. */
export function hasAnchorPositioning(view: Window & typeof globalThis): boolean {
  return 'CSS' in view && view.CSS.supports('anchor-name', '--glz');
}

/**
 * Positions the open popover `panel` on `side` of `anchor`: with CSS anchor positioning where
 * the browser has it (flipping to the other side when there is no room), else by measuring.
 * `name` is a unique dashed ident for the anchor. Call after `showPopover()`, and again when
 * the page scrolls or resizes in browsers without anchor positioning.
 */
export function positionPopover(
  anchor: HTMLElement,
  panel: HTMLElement,
  side: GlzSide,
  name: string,
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
    panel.style.setProperty('inset', 'auto');
    return;
  }
  const rect = anchor.getBoundingClientRect();
  const size = panel.getBoundingClientRect();
  const at = placePopover(
    { top: rect.top, left: rect.left, width: rect.width, height: rect.height },
    { width: size.width, height: size.height },
    { width: view.innerWidth, height: view.innerHeight },
    side,
  );
  panel.style.setProperty('inset', `${String(at.top)}px auto auto ${String(at.left)}px`);
}
