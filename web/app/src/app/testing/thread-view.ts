import { BreakpointObserver } from '@angular/cdk/layout';
import { TestBed } from '@angular/core/testing';
import { provideRouter } from '@angular/router';
import type { TurnStartParams } from '@glave/client';
import { of } from 'rxjs';

import { ThreadComponent } from '../components/thread/thread.component';
import { FirstTurnService } from '../services/first-turn.service';
import { ProjectsService } from '../services/projects.service';
import { type FakeSession, provideFakeSession } from './fake-session';

/** The thread view, opened, as a test reaches it. */
export interface OpenThread {
  readonly root: HTMLElement;
  readonly fixture: ReturnType<typeof TestBed.createComponent<ThreadComponent>>;
}

/**
 * Opens thread-1 in the thread view on a wide screen, against `fake`, after the new chat sent
 * it `first`, if given; once what it asked as it opened has been answered.
 */
export async function openThread(fake: FakeSession, first?: TurnStartParams): Promise<OpenThread> {
  const breakpoints = { observe: () => of({ matches: false, breakpoints: {} }) };
  TestBed.configureTestingModule({
    providers: [
      provideRouter([]),
      ...provideFakeSession(fake),
      { provide: BreakpointObserver, useValue: breakpoints },
    ],
  });
  await TestBed.inject(ProjectsService).refresh();
  if (first !== undefined) {
    TestBed.inject(FirstTurnService).start(first);
  }
  const fixture = TestBed.createComponent(ThreadComponent);
  fixture.componentRef.setInput('thread', 'thread-1');
  await fixture.whenStable();
  await new Promise((resolve) => setTimeout(resolve));
  await fixture.whenStable();
  return { root: fixture.nativeElement as HTMLElement, fixture };
}
