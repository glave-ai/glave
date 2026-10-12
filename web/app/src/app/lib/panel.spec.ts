import { PANEL_DEFAULT, SIDE_NAV_DEFAULT, toPaneState, toPanelState } from './panel';

describe('toPaneState', () => {
  it('reads back what was stored, and the fallback for anything wrong', () => {
    expect(toPaneState({ open: false, width: 301.6 }, SIDE_NAV_DEFAULT)).toEqual({
      open: false,
      width: 302,
    });
    expect(toPaneState(null, SIDE_NAV_DEFAULT)).toEqual(SIDE_NAV_DEFAULT);
    expect(toPaneState({ open: 1, width: 0 }, SIDE_NAV_DEFAULT)).toEqual(SIDE_NAV_DEFAULT);
  });
});

describe('toPanelState', () => {
  it('reads back what was stored', () => {
    expect(toPanelState({ open: true, width: 412.4, tab: 'changes' })).toEqual({
      open: true,
      width: 412,
      tab: 'changes',
    });
  });

  it('takes the default for anything missing or wrong', () => {
    expect(toPanelState(null)).toEqual(PANEL_DEFAULT);
    expect(toPanelState('open')).toEqual(PANEL_DEFAULT);
    expect(toPanelState({ open: 'yes', width: -3, tab: 'terminal' })).toEqual(PANEL_DEFAULT);
    expect(toPanelState({ width: Number.POSITIVE_INFINITY })).toEqual(PANEL_DEFAULT);
  });
});
