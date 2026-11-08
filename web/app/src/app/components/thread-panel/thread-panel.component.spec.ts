import { TestBed } from '@angular/core/testing';
import { provideRouter } from '@angular/router';
import type { WorkspaceBound } from '@glave/client';
import { expectAccessible } from '@glave/glaze/testing';
import { fakeMedia } from '@glave/glaze/testing/media';

import { ChangedFilesService } from '../../services/changed-files.service';
import { CommandsService } from '../../services/commands.service';
import { PanelService } from '../../services/panel.service';
import { PlanService } from '../../services/plan.service';
import { ProjectsService } from '../../services/projects.service';
import { ThreadPanelService } from '../../services/thread-panel.service';
import { ThreadService } from '../../services/thread.service';
import { FakeSession, provideFakeSession } from '../../testing/fake-session';
import { command, event, record, thread } from '../../testing/fixtures';
import { ThreadPanelComponent } from './thread-panel.component';

/** A turn that changed a file and keeps a dev server running. */
const WORKED = [
  event(1, 'tool.call.requested', {
    call_id: 'c2',
    tool: 'edit',
    arguments: { json: { path: 'src/args.rs', old_string: 'x', new_string: 'a\nb\nc' } },
  }),
  event(2, 'tool.call.completed', { call_id: 'c2', is_error: false, output: [] }),
  event(3, 'command.started', { command: 'd1', script: 'npm run dev' }),
  event(4, 'turn.ended', { outcome: 'completed' }),
];

const FOLDER: WorkspaceBound = {
  workspace: 'w1',
  computer: { id: 'c1', kind: 'worker' },
  opened: { kind: 'folder', path: '/work/glave' },
  root: '/work/glave',
};

const FILES = [
  {
    path: 'src/args.rs',
    added: 3,
    removed: 1,
    changes: [
      {
        kind: 'edited',
        lines: [
          { kind: 'removed', text: 'x' },
          { kind: 'added', text: 'a' },
        ],
      },
    ],
  },
] as const;

async function render(
  fake: FakeSession,
  events: readonly unknown[],
  narrow = false,
): Promise<{
  readonly root: HTMLElement;
  readonly fixture: ReturnType<typeof TestBed.createComponent<ThreadPanelComponent>>;
}> {
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
  await TestBed.inject(ProjectsService).refresh();
  await TestBed.inject(ThreadService).open('thread-1');
  const fixture = TestBed.createComponent(ThreadPanelComponent);
  await settle(fixture);
  return { root: fixture.nativeElement as HTMLElement, fixture };
}

async function settle(fixture: { whenStable: () => Promise<unknown> }): Promise<void> {
  await fixture.whenStable();
  await new Promise((resolve) => setTimeout(resolve));
  await fixture.whenStable();
}

function tabs(root: HTMLElement): readonly HTMLButtonElement[] {
  return Array.from(root.querySelectorAll<HTMLButtonElement>('[data-testid="panel-tabs-tab"]'));
}

function worked(): FakeSession {
  return new FakeSession()
    .answer('thread/get', { thread: thread('thread-1', 'Add --verbose', { workspace: FOLDER }) })
    .answer('command/list', { commands: [command('d1', 'npm run dev')] })
    .answer('command/stop', {})
    .answer('thread/changes', {
      changes: { files: FILES, turns: [], note: 'Changes made by commands are not listed.' },
    });
}

describe('ThreadPanelComponent', () => {
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it('opens on Overview, and shows the files the thread changed in full in Changes', async () => {
    const { root, fixture } = await render(worked(), WORKED);
    expect(tabs(root).map((tab) => [tab.textContent.trim(), tab.ariaSelected])).toEqual([
      ['Overview', 'true'],
      ['Changes', 'false'],
    ]);
    expect(root.querySelector('[role="tablist"]')?.getAttribute('aria-label')).toBe(
      'What this thread shows',
    );
    expect(root.querySelector('app-thread-overview')).not.toBeNull();
    await expectAccessible(root);
    tabs(root)[1]?.click();
    await fixture.whenStable();
    expect(root.querySelector('app-thread-overview')).toBeNull();
    expect(root.querySelector('[data-testid="file"]')?.textContent).toContain('src/args.rs');
    expect(root.querySelector('[data-testid="file-change"]')?.textContent).toContain('a');
    expect(root.querySelector('[data-testid="files-note"]')?.textContent).toBe(
      'Changes made by commands are not listed.',
    );
    await expectAccessible(root);
  });

  it('has no Changes tab before the thread changed files, and Overview again in each thread', async () => {
    const fake = worked();
    const give = fake.held('thread/changes');
    const { root, fixture } = await render(fake, WORKED);
    expect(tabs(root).map((tab) => tab.textContent.trim())).toEqual(['Overview']);
    give({ changes: { files: FILES, turns: [], note: '' } });
    await settle(fixture);
    tabs(root)[1]?.click();
    await fixture.whenStable();
    expect(tabs(root)[1]?.ariaSelected).toBe('true');
    void TestBed.inject(ThreadService).open('thread-2');
    await fixture.whenStable();
    expect(tabs(root)[0]?.ariaSelected).toBe('true');
  });

  it('heads itself one level up from its sections, heard alone beside the page', async () => {
    const { root } = await render(worked(), WORKED);
    const heading = root.querySelector('h2');
    expect(heading?.textContent.trim()).toBe('This thread');
    expect(heading?.classList).toContain('glz-visually-hidden');
    expect(root.querySelectorAll('h3').length).toBeGreaterThan(0);
    const close = root.querySelector<HTMLElement>('[data-testid="panel-close"]');
    expect(close === null ? null : getComputedStyle(close).display).toBe('none');
  });

  it('gives the keyboard’s place to its heading when Stop takes the last command away', async () => {
    const { root, fixture } = await render(worked(), WORKED);
    const stop = root.querySelector<HTMLButtonElement>('[data-testid="command-stop"]');
    stop?.focus();
    expect(document.activeElement).toBe(stop);
    stop?.click();
    await settle(fixture);
    expect(root.querySelector('[data-testid="command-stop"]')).toBeNull();
    expect(document.activeElement).toBe(root.querySelector('[data-testid="panel-heading"]'));
  });

  it('as a sheet, shows its heading and closes', async () => {
    const { root, fixture } = await render(worked(), WORKED, true);
    const panel = TestBed.inject(PanelService);
    panel.setShowing(true);
    await fixture.whenStable();
    expect(root.querySelector('h2')?.classList).not.toContain('glz-visually-hidden');
    root.querySelector<HTMLButtonElement>('[data-testid="panel-close"]')?.click();
    expect(panel.showing()).toBe(false);
  });
});
