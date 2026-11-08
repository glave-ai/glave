import { TestBed } from '@angular/core/testing';
import { provideRouter } from '@angular/router';
import { fakeMedia } from '@glave/glaze/testing/media';

import { ChangedFilesService } from '../services/changed-files.service';
import { CommandsService } from '../services/commands.service';
import { PlanService } from '../services/plan.service';
import { ProjectsService } from '../services/projects.service';
import { ThreadPanelService } from '../services/thread-panel.service';
import { ThreadService } from '../services/thread.service';
import { type FakeSession, provideFakeSession } from './fake-session';
import { record } from './fixtures';

/**
 * Opens thread-1, whose history is `events`, in project-1, called glave, with what a thread's
 * panel reads beside it, against `fake`: on a narrow screen when `narrow`, its project's name
 * read first unless `read` is false.
 */
export async function openForPanel(
  fake: FakeSession,
  events: readonly unknown[],
  { narrow = false, read = true }: { readonly narrow?: boolean; readonly read?: boolean } = {},
): Promise<void> {
  localStorage.clear();
  fake
    .answer('thread/events', { events })
    .answer('project/list', { projects: [record('project-1', 'glave')], gone: [] })
    .answer('thread/list', { threads: [] });
  fakeMedia(narrow);
  TestBed.configureTestingModule({
    providers: [
      ...provideFakeSession(fake),
      provideRouter([]),
      ThreadService,
      CommandsService,
      ChangedFilesService,
      PlanService,
      ThreadPanelService,
    ],
  });
  if (read) {
    await TestBed.inject(ProjectsService).refresh();
  }
  await TestBed.inject(ThreadService).open('thread-1');
}

/** Lets what `fixture` asked as it was drawn be answered, and draws it again. */
export async function drawn(fixture: {
  readonly whenStable: () => Promise<unknown>;
}): Promise<void> {
  await fixture.whenStable();
  await new Promise((resolve) => setTimeout(resolve));
  await fixture.whenStable();
}
