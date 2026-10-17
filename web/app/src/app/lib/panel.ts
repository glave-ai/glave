/** The tabs of the thread's tools panel, in order. */
export const TOOL_TABS = ['tasks', 'changes', 'commands'] as const;
export type ToolTab = (typeof TOOL_TABS)[number];

/** How the person left a side panel: open or collapsed, and its width. */
export interface PaneState {
  readonly open: boolean;
  /** In pixels; `null` for Glaze's default width. */
  readonly width: number | null;
}

/** How the person left the tools panel: also its tab. */
export interface PanelState extends PaneState {
  readonly tab: ToolTab;
}

/** Open, at the default width: how the side nav starts for someone new. */
export const SIDE_NAV_DEFAULT: PaneState = { open: true, width: null };

/** Closed, at the default width, on its first tab: how the panel starts for someone new. */
export const PANEL_DEFAULT: PanelState = { open: false, width: null, tab: 'tasks' };

function fields(value: unknown): Partial<Record<keyof PanelState, unknown>> {
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

/** A stored tools panel state, read back: whatever is missing or wrong is the default's. */
export function toPanelState(value: unknown): PanelState {
  const tab = TOOL_TABS.find((each) => each === fields(value).tab);
  return { ...toPaneState(value, PANEL_DEFAULT), tab: tab ?? PANEL_DEFAULT.tab };
}
