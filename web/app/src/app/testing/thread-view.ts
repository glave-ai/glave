import { BreakpointObserver } from '@angular/cdk/layout';
import { TestBed } from '@angular/core/testing';
import { provideRouter } from '@angular/router';
import type { TurnStartParams } from '@glave/client';
import { of } from 'rxjs';

import { ThreadComponent } from '../components/thread/thread.component';
import { FirstTurnService } from '../services/first-turn.service';
import { ProjectsService } from '../services/projects.service';
import { FakeSession, provideFakeSession } from './fake-session';
import { MODELS, SERVER, changed, record, thread } from './fixtures';

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

/** A session that answers thread-1's history with `events` and what it changed with `changes`. */
export function session(events: readonly unknown[], changes = changed(1, [])): FakeSession {
  return new FakeSession()
    .answer('thread/events', { events })
    .answer('thread/changes', changes)
    .answer('model/list', { models: MODELS })
    .answer('server/discover', SERVER)
    .answer('schedule/list', { schedules: [] })
    .answer('tasks/list', { tasks: [] })
    .answer('project/list', { projects: [record('p1', 'glave')] })
    .answer('thread/list', { threads: [thread('thread-1', 'Tidy the repo', { project: 'p1' })] });
}

/** Waits a task, for what is already answered to come in. */
export const settle = (): Promise<unknown> => new Promise((resolve) => setTimeout(resolve));

/** Types `text` into the thread's composer and sends it. */
export async function type(root: HTMLElement, text: string): Promise<void> {
  const input = root.querySelector<HTMLTextAreaElement>('[data-testid="thread-composer-input"]');
  if (input) {
    input.value = text;
    input.dispatchEvent(new Event('input'));
    input.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', cancelable: true }));
  }
  await settle();
}
