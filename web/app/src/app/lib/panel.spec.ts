import { PANEL_DEFAULT, SIDE_NAV_DEFAULT, toPanelState, toSideNavState } from './panel';

describe('toSideNavState', () => {
  it('reads back what was stored, and the default for anything wrong', () => {
    expect(toSideNavState({ open: false, width: 301.6 })).toEqual({ open: false, width: 302 });
    expect(toSideNavState(null)).toEqual(SIDE_NAV_DEFAULT);
    expect(toSideNavState({ open: 1, width: 0 })).toEqual(SIDE_NAV_DEFAULT);
  });
});

describe('toPanelState', () => {
  it('reads back what was stored', () => {
    expect(toPanelState({ open: true, width: 412.4, tab: 'files' })).toEqual({
      open: true,
      width: 412,
      tab: 'files',
    });
  });

  it('takes the default for anything missing or wrong', () => {
    expect(toPanelState(null)).toEqual(PANEL_DEFAULT);
    expect(toPanelState('open')).toEqual(PANEL_DEFAULT);
    expect(toPanelState({ open: 'yes', width: -3, tab: 'terminal' })).toEqual(PANEL_DEFAULT);
    expect(toPanelState({ width: Number.POSITIVE_INFINITY })).toEqual(PANEL_DEFAULT);
  });
});
