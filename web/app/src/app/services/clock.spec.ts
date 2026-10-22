import { type Signal, signal } from '@angular/core';
import { TestBed } from '@angular/core/testing';

import { TICK_MS, clock } from './clock';

describe('clock', () => {
  beforeEach(() => {
    vi.useFakeTimers({ toFake: ['Date', 'setInterval', 'clearInterval'] });
    vi.setSystemTime(Date.parse('2026-10-11T10:00:00.000Z'));
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it('moves on each second while it runs, rests while it does not, and goes with what made it', () => {
    const running = signal(false);
    const now: Signal<number> = TestBed.runInInjectionContext(() => clock(running));
    const start = Date.now();
    TestBed.tick();
    expect(vi.getTimerCount()).toBe(0);
    vi.advanceTimersByTime(TICK_MS * 3);
    expect(now()).toBe(start);
    running.set(true);
    TestBed.tick();
    // Starting, it reads the time at once.
    expect(now()).toBe(start + TICK_MS * 3);
    vi.advanceTimersByTime(TICK_MS);
    expect(now()).toBe(start + TICK_MS * 4);
    running.set(false);
    TestBed.tick();
    expect(vi.getTimerCount()).toBe(0);
    running.set(true);
    TestBed.tick();
    expect(vi.getTimerCount()).toBe(1);
    TestBed.resetTestingModule();
    expect(vi.getTimerCount()).toBe(0);
  });
});
