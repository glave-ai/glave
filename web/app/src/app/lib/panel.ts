/** The tabs of the thread's tools panel, in order. */
export const TOOL_TABS = ['tasks', 'changes'] as const;
export type ToolTab = (typeof TOOL_TABS)[number];

/** How the person left the side nav: open or collapsed, and its width. */
export interface SideNavState {
  readonly open: boolean;
  /** In pixels; `null` for Glaze's default width. */
  readonly width: number | null;
}

/** How the person left the tools panel: open or not, its width, and its tab. */
export interface PanelState extends SideNavState {
  readonly tab: ToolTab;
}

/** Open, at the default width: how the side nav starts for someone new. */
export const SIDE_NAV_DEFAULT: SideNavState = { open: true, width: null };

/** Closed, at the default width, on its first tab: how the panel starts for someone new. */
export const PANEL_DEFAULT: PanelState = { open: false, width: null, tab: 'tasks' };

function fields(value: unknown): Partial<Record<keyof PanelState, unknown>> {
  return typeof value === 'object' && value !== null ? value : {};
}

/** A stored width, read back: a positive number of whole pixels, else `null`. */
function storedWidth(width: unknown): number | null {
  return typeof width === 'number' && Number.isFinite(width) && width > 0
    ? Math.round(width)
    : null;
}

/** A stored side nav state, read back: whatever is missing or wrong is the default's. */
export function toSideNavState(value: unknown): SideNavState {
  const stored = fields(value);
  return {
    open: typeof stored.open === 'boolean' ? stored.open : SIDE_NAV_DEFAULT.open,
    width: storedWidth(stored.width),
  };
}

/** A stored panel state, read back: whatever is missing or wrong is the default's. */
export function toPanelState(value: unknown): PanelState {
  const stored = fields(value);
  const tab = TOOL_TABS.find((each) => each === stored.tab);
  return {
    open: typeof stored.open === 'boolean' ? stored.open : PANEL_DEFAULT.open,
    width: storedWidth(stored.width),
    tab: tab ?? PANEL_DEFAULT.tab,
  };
}
