import { Injector, runInInjectionContext, signal } from '@angular/core';
import { TestBed } from '@angular/core/testing';

import { closedWhileNarrow } from './narrow';

describe('closedWhileNarrow', () => {
  it('closes on a narrow screen, and is as it was once the screen is wide again', () => {
    const narrow = signal(false);
    const open = signal(true);
    runInInjectionContext(TestBed.inject(Injector), () => {
      closedWhileNarrow(narrow, open);
    });
    TestBed.tick();
    expect(open()).toBe(true);
    narrow.set(true);
    TestBed.tick();
    expect(open()).toBe(false);
    // Opened while narrow, it may be closed again; the wide state is kept apart.
    open.set(true);
    TestBed.tick();
    expect(open()).toBe(true);
    narrow.set(false);
    TestBed.tick();
    expect(open()).toBe(true);
    open.set(false);
    narrow.set(true);
    TestBed.tick();
    narrow.set(false);
    TestBed.tick();
    expect(open()).toBe(false);
  });
});
