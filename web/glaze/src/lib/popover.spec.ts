import { hasAnchorPositioning, placePopover, positionPopover } from './popover';

const VIEWPORT = { width: 800, height: 600 };
const PANEL = { width: 100, height: 40 };

describe('placePopover', () => {
  it('centres the panel on the side asked', () => {
    const anchor = { top: 300, left: 350, width: 100, height: 20 };
    expect(placePopover(anchor, PANEL, VIEWPORT, 'above')).toEqual({ top: 256, left: 350 });
    expect(placePopover(anchor, PANEL, VIEWPORT, 'below')).toEqual({ top: 324, left: 350 });
    expect(placePopover(anchor, PANEL, VIEWPORT, 'end')).toEqual({ top: 290, left: 454 });
    expect(placePopover(anchor, PANEL, VIEWPORT, 'start')).toEqual({ top: 290, left: 246 });
  });

  it('flips to the other side when there is no room', () => {
    const top = { top: 10, left: 350, width: 100, height: 20 };
    expect(placePopover(top, PANEL, VIEWPORT, 'above').top).toBe(34);
  });

  it('keeps the panel inside the viewport', () => {
    const corner = { top: 300, left: 0, width: 20, height: 20 };
    expect(placePopover(corner, PANEL, VIEWPORT, 'above').left).toBe(0);
  });
});

describe('positionPopover', () => {
  function elements(): readonly [HTMLElement, HTMLElement] {
    const anchor = document.createElement('button');
    const panel = document.createElement('div');
    anchor.getBoundingClientRect = () => new DOMRect(300, 300, 100, 20);
    panel.getBoundingClientRect = () => new DOMRect(0, 0, 100, 40);
    return [anchor, panel];
  }

  afterEach(() => {
    vi.restoreAllMocks();
  });

  it('measures and places the panel where anchor positioning is missing', () => {
    vi.spyOn(CSS, 'supports').mockReturnValue(false);
    expect(hasAnchorPositioning(window)).toBe(false);
    const [anchor, panel] = elements();
    positionPopover(anchor, panel, 'below', '--glz-test');
    expect(panel.style.getPropertyValue('position')).toBe('fixed');
    expect(panel.style.getPropertyValue('inset')).toBe('324px auto auto 300px');
  });

  it('leaves placing to CSS anchor positioning where the browser has it', () => {
    vi.spyOn(CSS, 'supports').mockReturnValue(true);
    const [anchor, panel] = elements();
    positionPopover(anchor, panel, 'above', '--glz-test');
    expect(anchor.style.getPropertyValue('anchor-name')).toBe('--glz-test');
    expect(panel.style.getPropertyValue('position-anchor')).toBe('--glz-test');
    expect(panel.style.getPropertyValue('position-area')).toBe('block-start');
  });
});
