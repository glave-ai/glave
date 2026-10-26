/** How the person left a side panel: open or collapsed, and its width. */
export interface PaneState {
  readonly open: boolean;
  /** In pixels; `null` for Glaze's default width. */
  readonly width: number | null;
}

/** Open, at the default width: how the side nav starts for someone new. */
export const SIDE_NAV_DEFAULT: PaneState = { open: true, width: null };

/** Open, at the default width: how a thread's panel starts for someone new. */
export const PANEL_DEFAULT: PaneState = { open: true, width: null };

function fields(value: unknown): Partial<Record<keyof PaneState, unknown>> {
  return typeof value === 'object' && value !== null ? value : {};
}

/** A stored state, read back: whatever is missing or wrong is `fallback`'s. */
export function toPaneState(value: unknown, fallback: PaneState): PaneState {
  const { open, width } = fields(value);
  return {
    open: typeof open === 'boolean' ? open : fallback.open,
    // A positive number of whole pixels.
    width:
      typeof width === 'number' && Number.isFinite(width) && width > 0
        ? Math.round(width)
        : fallback.width,
  };
}
