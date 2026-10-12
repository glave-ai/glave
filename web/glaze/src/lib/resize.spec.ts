import { clampWidth, resizedWidth } from './resize';

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

describe('resizedWidth', () => {
  it('widens a panel at the end as its edge moves toward the start', () => {
    expect(resizedWidth(400, -50, 'ltr', 280, 720)).toBe(450);
    expect(resizedWidth(400, 50, 'ltr', 280, 720)).toBe(350);
    expect(resizedWidth(400, 50, 'rtl', 280, 720)).toBe(450);
  });

  it('stops at the bounds', () => {
    expect(resizedWidth(400, -1000, 'ltr', 280, 720)).toBe(720);
    expect(resizedWidth(400, 1000, 'ltr', 280, 720)).toBe(280);
  });
});
