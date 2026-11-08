import { ApplicationRef } from '@angular/core';
import { type ComponentFixture, TestBed } from '@angular/core/testing';
import { firstValueFrom } from 'rxjs';

/** More turns than anything a test draws takes to come to rest; past them, it keeps changing. */
const TURNS = 20;

/**
 * Lets what a test started come to rest: at rest once a turn ends with nothing more to draw.
 * Each turn draws what is changed, then gives what that started (a fake's answers, a file read) a
 * task to come back in. Turns are counted, never timed, so a busy machine makes a test slower,
 * never wrong. Given its `fixture`, an error while it draws fails the test here; a service tested
 * alone has none. What no turn brings back, such as code loaded late, the test awaits first.
 */
export async function settle(fixture?: ComponentFixture<unknown>): Promise<void> {
  const app = TestBed.inject(ApplicationRef);
  for (let turn = 0; turn < TURNS; turn += 1) {
    await (fixture ?? app).whenStable();
    await new Promise((resolve) => setTimeout(resolve));
    if (await firstValueFrom(app.isStable)) {
      return;
    }
  }
  throw new Error(`what the test started was still changing after ${String(TURNS)} turns`);
}
