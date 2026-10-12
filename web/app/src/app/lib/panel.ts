/** The tabs of the thread's tools panel, in order. */
export const TOOL_TABS = ['tasks', 'files'] as const;
export type ToolTab = (typeof TOOL_TABS)[number];

/** How the person left the tools panel: open or not, its width, and its tab. */
export interface PanelState {
  readonly open: boolean;
  /** In pixels; `null` for Glaze's default width. */
  readonly width: number | null;
  readonly tab: ToolTab;
}

/** Closed, at the default width, on its first tab: how the panel starts for someone new. */
export const PANEL_DEFAULT: PanelState = { open: false, width: null, tab: 'tasks' };

/** A stored panel state, read back: whatever is missing or wrong is the default's. */
export function toPanelState(value: unknown): PanelState {
  if (typeof value !== 'object' || value === null) {
    return PANEL_DEFAULT;
  }
  const stored: Partial<Record<keyof PanelState, unknown>> = value;
  const tab = TOOL_TABS.find((each) => each === stored.tab);
  return {
    open: typeof stored.open === 'boolean' ? stored.open : PANEL_DEFAULT.open,
    width:
      typeof stored.width === 'number' && Number.isFinite(stored.width) && stored.width > 0
        ? Math.round(stored.width)
        : PANEL_DEFAULT.width,
    tab: tab ?? PANEL_DEFAULT.tab,
  };
}
