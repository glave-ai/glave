import { clampWidth, draggedSplit, splitterKeys } from './resize';

describe('clampWidth', () => {
  it('keeps a width between the bounds, in whole pixels', () => {
    expect(clampWidth(100, 280, 720)).toBe(280);
    expect(clampWidth(900, 280, 720)).toBe(720);
    expect(clampWidth(400.6, 280, 720)).toBe(401);
  });

  it('lets the minimum win when the bounds cross', () => {
    expect(clampWidth(500, 300, 200)).toBe(300);
  });
});

describe('draggedSplit', () => {
  it('widens a panel as its edge moves away from its side, within the bounds', () => {
    expect(draggedSplit(400, -50, 'end', 'ltr', 280, 720)).toEqual({ open: true, width: 450 });
    expect(draggedSplit(400, 50, 'end', 'ltr', 280, 720)).toEqual({ open: true, width: 350 });
    expect(draggedSplit(400, 50, 'end', 'rtl', 280, 720)).toEqual({ open: true, width: 450 });
    expect(draggedSplit(300, 40, 'start', 'ltr', 200, 480)).toEqual({ open: true, width: 340 });
    expect(draggedSplit(300, -40, 'start', 'rtl', 200, 480)).toEqual({ open: true, width: 340 });
    expect(draggedSplit(400, -1000, 'end', 'ltr', 280, 720)).toEqual({ open: true, width: 720 });
  });

  it('stops at the minimum, collapses below half of it, and opens again past it', () => {
    expect(draggedSplit(300, -150, 'start', 'ltr', 200, 480)).toEqual({ open: true, width: 200 });
    expect(draggedSplit(300, -220, 'start', 'ltr', 200, 480)).toEqual({ open: false });
    expect(draggedSplit(400, 300, 'end', 'ltr', 280, 720)).toEqual({ open: false });
    expect(draggedSplit(0, 120, 'start', 'ltr', 200, 480)).toEqual({ open: true, width: 200 });
    expect(draggedSplit(0, -60, 'end', 'ltr', 280, 720)).toEqual({ open: false });
  });
});

describe('splitterKeys', () => {
  it('widens with the arrow pointing away from the panel', () => {
    expect(splitterKeys('start', 'ltr')).toEqual({ grow: 'ArrowRight', shrink: 'ArrowLeft' });
    expect(splitterKeys('end', 'ltr')).toEqual({ grow: 'ArrowLeft', shrink: 'ArrowRight' });
    expect(splitterKeys('start', 'rtl')).toEqual({ grow: 'ArrowLeft', shrink: 'ArrowRight' });
    expect(splitterKeys('end', 'rtl')).toEqual({ grow: 'ArrowRight', shrink: 'ArrowLeft' });
  });
});
