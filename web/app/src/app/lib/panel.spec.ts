import { PANEL_DEFAULT, toPanelState } from './panel';

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
