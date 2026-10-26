import { PANEL_DEFAULT, SIDE_NAV_DEFAULT, toPaneState } from './panel';

describe('toPaneState', () => {
  it('reads back what was stored, and the fallback for anything wrong', () => {
    expect(toPaneState({ open: false, width: 301.6 }, SIDE_NAV_DEFAULT)).toEqual({
      open: false,
      width: 302,
    });
    expect(toPaneState(null, SIDE_NAV_DEFAULT)).toEqual(SIDE_NAV_DEFAULT);
    expect(toPaneState({ open: 1, width: 0 }, SIDE_NAV_DEFAULT)).toEqual(SIDE_NAV_DEFAULT);
  });

  it('starts a thread’s panel open, whatever is missing or wrong', () => {
    expect(PANEL_DEFAULT.open).toBe(true);
    expect(toPaneState('open', PANEL_DEFAULT)).toEqual(PANEL_DEFAULT);
    expect(toPaneState({ open: 'yes', width: -3 }, PANEL_DEFAULT)).toEqual(PANEL_DEFAULT);
    expect(toPaneState({ width: Number.POSITIVE_INFINITY }, PANEL_DEFAULT)).toEqual(PANEL_DEFAULT);
  });
});
