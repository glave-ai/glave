import { fakeMotion } from '../testing/motion';
import { countUp, durationOf } from './motion';

describe('durationOf', () => {
  afterEach(() => {
    document.documentElement.style.removeProperty('--glz-duration-slow');
  });

  it('reads a duration from its token, in milliseconds', () => {
    const root = document.documentElement;
    root.style.setProperty('--glz-duration-slow', '320ms');
    expect(durationOf(document, 'slow')).toBe(320);
    root.style.setProperty('--glz-duration-slow', '0.5s');
    expect(durationOf(document, 'slow')).toBe(500);
    // Less motion, and a page without the tokens: nothing moves.
    root.style.setProperty('--glz-duration-slow', '0ms');
    expect(durationOf(document, 'slow')).toBe(0);
    root.style.removeProperty('--glz-duration-slow');
    expect(durationOf(document, 'slow')).toBe(0);
  });
});

describe('countUp', () => {
  it('counts up eased, frame by frame, and ends on the number', () => {
    const motion = fakeMotion('320ms');
    const steps: number[] = [];
    countUp(document, 10, 30, (value) => steps.push(value));
    motion.to(0.5);
    motion.to(0.9);
    motion.end();
    expect(steps).toEqual([20, 28, 30]);
    motion.restore();
  });

  it('stops where it is when stopped', () => {
    const motion = fakeMotion('320ms');
    const steps: number[] = [];
    const stop = countUp(document, 0, 100, (value) => steps.push(value));
    motion.to(0.25);
    stop();
    motion.to(0.75);
    expect(steps).toEqual([25]);
    motion.restore();
  });

  it('shows the number at once when the person asks for less motion', () => {
    const motion = fakeMotion('0ms');
    const steps: number[] = [];
    countUp(document, 10, 30, (value) => steps.push(value));
    expect(steps).toEqual([30]);
    motion.restore();
  });
});
