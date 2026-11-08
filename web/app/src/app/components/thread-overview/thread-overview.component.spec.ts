import { TestBed } from '@angular/core/testing';
import { provideRouter } from '@angular/router';
import { ApiError, ErrorCode, type ThreadSummary, type WorkspaceBound } from '@glave/client';
import { expectAccessible } from '@glave/glaze/testing';
import { fakeMedia } from '@glave/glaze/testing/media';

import { ChangedFilesService } from '../../services/changed-files.service';
import { CommandsService } from '../../services/commands.service';
import { PlanService } from '../../services/plan.service';
import { ProjectsService } from '../../services/projects.service';
import { ThreadPanelService } from '../../services/thread-panel.service';
import { ThreadService } from '../../services/thread.service';
import { FakeSession, provideFakeSession } from '../../testing/fake-session';
import { command, event, record, thread } from '../../testing/fixtures';
import { ThreadOverviewComponent } from './thread-overview.component';

/** A turn that changed two files, keeps a dev server running and has a plan. */
const WORKED = [
  event(1, 'message.added', { role: 'user', parts: [{ type: 'text', text: 'Add --verbose' }] }),
  event(2, 'tool.call.requested', {
    call_id: 'c2',
    tool: 'edit',
    arguments: { json: { path: 'src/args.rs', old_string: 'x', new_string: 'a\nb\nc' } },
  }),
  event(3, 'tool.call.completed', { call_id: 'c2', is_error: false, output: [] }),
  event(4, 'command.started', { command: 'd1', script: 'npm run dev' }),
  event(5, 'plan.updated', { steps: [], call_id: 'c4' }),
  event(6, 'message.added', { role: 'assistant', parts: [{ type: 'text', text: 'Done.' }] }),
  event(7, 'turn.ended', { outcome: 'completed' }),
];

const WORKTREE: WorkspaceBound = {
  workspace: 'w1',
  computer: { id: 'c1', kind: 'worker' },
  opened: { kind: 'worktree', repository: '/work/glave', branch: 'glave/0123456789ab' },
  root: '/worktrees/glave-0123',
};

const CONTEXT = { percent: 39, tokens: 49_920, window: 128_000 };

/** A connector of the project's that needs signing in again. */
const EXPIRED = {
  id: 'linear',
  name: 'Linear',
  status: 'sign_in_again',
  categories: [],
  tags: [],
  company: 'linear',
  description: '',
  docs: '',
  url: 'https://mcp.linear.app/mcp',
  returns: 'text',
  sign_in: 'dcr',
} as const;

async function render(
  fake: FakeSession,
  events: readonly unknown[],
  read = true,
): Promise<{
  readonly root: HTMLElement;
  readonly fixture: ReturnType<typeof TestBed.createComponent<ThreadOverviewComponent>>;
}> {
  localStorage.clear();
  fake
    .answer('thread/events', { events })
    .answer('project/list', { projects: [record('project-1', 'glave')], gone: [] })
    .answer('thread/list', { threads: [] });
  fakeMedia(false);
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
  const fixture = TestBed.createComponent(ThreadOverviewComponent);
  await fixture.whenStable();
  await new Promise((resolve) => setTimeout(resolve));
  await fixture.whenStable();
  return { root: fixture.nativeElement as HTMLElement, fixture };
}

function texts(root: HTMLElement, testId: string): readonly string[] {
  return Array.from(root.querySelectorAll(`[data-testid="${testId}"]`)).map((each) =>
    each.textContent.replace(/\s+/gu, ' ').trim(),
  );
}

function summary(overrides: Partial<ThreadSummary> = {}): ThreadSummary {
  return thread('thread-1', 'Add a --verbose flag', {
    workspace: WORKTREE,
    context: CONTEXT,
    ...overrides,
  });
}

function worked(overrides: Partial<ThreadSummary> = {}): FakeSession {
  return new FakeSession()
    .answer('thread/get', { thread: summary(overrides) })
    .answer('command/list', { commands: [command('d1', 'npm run dev')] })
    .answer('connector/list', { connectors: [EXPIRED], categories: [], companies: [], tags: [] })
    .answer('thread/plan', {
      plan: {
        proposed: false,
        sentence: 'Plan: 1 of 2 steps done',
        steps: [
          { step: 'Read how flags are parsed', status: 'completed' },
          { step: 'Add --verbose', status: 'in_progress' },
        ],
      },
    })
    .answer('thread/changes', {
      changes: {
        files: [
          { path: 'src/args.rs', added: 3, removed: 1, changes: [] },
          { path: 'src/main.rs', added: 1, removed: 0, changes: [] },
        ],
        turns: [],
        note: 'Changes made by commands or other tools are not listed.',
      },
    });
}

describe('ThreadOverviewComponent', () => {
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it('shows where the thread works and each section that has something, a level down', async () => {
    const { root } = await render(worked(), WORKED);
    expect(texts(root, 'panel-where-part')).toEqual([
      'Another computer',
      'glave · /work/glave',
      'glave/0123456789ab · a separate copy',
    ]);
    const marks = root.querySelectorAll('[data-testid="panel-where-part"] glz-icon');
    expect(Array.from(marks).map((icon) => icon.getAttribute('aria-label'))).toEqual([
      'Computer',
      'Project',
      'Branch',
    ]);
    expect(texts(root, 'panel-plan-step')).toEqual(['Read how flags are parsed', 'Add --verbose']);
    expect(texts(root, 'panel-context')).toEqual(['39% used']);
    const meter = root.querySelector<HTMLMeterElement>('meter[data-testid="panel-meter"]');
    expect([meter?.value, meter?.max, meter?.getAttribute('aria-valuetext')]).toEqual([
      39,
      100,
      '39% used',
    ]);
    expect(texts(root, 'panel-compact')).toEqual(['Compact now']);
    const link = root.querySelector<HTMLAnchorElement>('[data-testid="panel-sign-in"]');
    expect(link?.textContent.trim()).toBe('1 needs signing in again');
    expect(link?.getAttribute('href')).toBe('/settings/connections?project=project-1');
    expect(texts(root, 'panel-changes').at(0)).toContain('4 lines added, 1 removed');
    expect(texts(root, 'panel-changes-count')).toEqual(['2 files']);
    expect(texts(root, 'command-text')).toEqual(['npm run dev']);
    expect(Array.from(root.querySelectorAll('h3')).map((each) => each.textContent.trim())).toEqual([
      'Where it works',
      'Plan',
      'Context',
      'Connections',
      'Changes',
      'Running in the background',
    ]);
    await expectAccessible(root);
  });

  it("keeps its facts from sight until the thread's project is known", async () => {
    const { root, fixture } = await render(worked(), WORKED, false);
    const facts = root.querySelector('[data-testid="panel-facts"]');
    expect(facts?.classList.contains('reading')).toBe(true);
    await TestBed.inject(ProjectsService).refresh();
    await fixture.whenStable();
    expect(facts?.classList.contains('reading')).toBe(false);
    expect(texts(root, 'panel-where-part')).toContain('glave · /work/glave');
  });

  it('shows its facts, with no project, when the lists hold no such project', async () => {
    const fake = worked();
    const { root, fixture } = await render(fake, WORKED, false);
    fake.answer('project/list', { projects: [], gone: [] });
    await TestBed.inject(ProjectsService).refresh();
    await fixture.whenStable();
    const facts = root.querySelector('[data-testid="panel-facts"]');
    expect(facts?.classList.contains('reading')).toBe(false);
    expect(texts(root, 'panel-where-part')).toEqual([
      'Another computer',
      'glave/0123456789ab · a separate copy',
    ]);
  });

  it('keeps the room of the files the thread changed, unseen, until they are read', async () => {
    const fake = worked();
    const give = fake.held('thread/changes');
    const ended = event(7, 'turn.ended', {
      outcome: 'completed',
      changed: ['src/args.rs', 'src/main.rs'],
    });
    const { root, fixture } = await render(fake, [...WORKED.slice(0, -1), ended]);
    const kept = root.querySelector<HTMLElement>('[data-testid="panel-changes"]');
    expect(kept?.classList).toContain('room');
    expect(getComputedStyle(kept ?? root).visibility).toBe('hidden');
    expect(texts(root, 'panel-changes-count')).toEqual(['2 files']);
    await expectAccessible(root);
    give({
      changes: {
        files: [{ path: 'src/args.rs', added: 3, removed: 1, changes: [] }],
        turns: [],
        note: '',
      },
    });
    await new Promise((resolve) => setTimeout(resolve));
    await fixture.whenStable();
    expect(kept?.classList).not.toContain('room');
    expect(texts(root, 'panel-changes-count')).toEqual(['1 file']);
  });

  it('names the point a clone is at, and a folder worked in place as no branch', async () => {
    const clone = await render(
      worked({
        workspace: {
          ...WORKTREE,
          opened: { kind: 'clone', url: 'https://github.com/acme/app.git', at: 'v1.2.0' },
        },
      }),
      WORKED,
    );
    expect(texts(clone.root, 'panel-where-part')).toEqual([
      'Another computer',
      'glave · /worktrees/glave-0123',
      'v1.2.0',
    ]);
    const marks = clone.root.querySelectorAll('[data-testid="panel-where-part"] glz-icon');
    expect(Array.from(marks).map((icon) => icon.getAttribute('aria-label'))).toContain(
      'Checked out at',
    );
    TestBed.resetTestingModule();
    const folder = await render(
      worked({ workspace: { ...WORKTREE, opened: { kind: 'folder', path: '/work/glave' } } }),
      WORKED,
    );
    expect(texts(folder.root, 'panel-where-part')).toEqual([
      'Another computer',
      'glave · /worktrees/glave-0123',
    ]);
  });

  it('compacts its context on request, and says why it could not', async () => {
    const fake = worked();
    const { root, fixture } = await render(fake, WORKED);
    const button = (): HTMLButtonElement | null =>
      root.querySelector<HTMLButtonElement>('[data-testid="panel-compact"]');
    fake.answer(
      'thread/compact',
      new ApiError({ code: ErrorCode.INVALID_PARAMS, message: 'the thread is busy' }),
    );
    button()?.click();
    await new Promise((resolve) => setTimeout(resolve));
    await fixture.whenStable();
    expect(texts(root, 'panel-compact-problem')).toEqual(['The thread is busy.']);
    expect(fake.paramsOf('thread/compact')).toEqual([{ thread: 'thread-1' }]);
    const give = fake.held('thread/compact');
    button()?.click();
    await fixture.whenStable();
    expect([button()?.textContent.trim(), button()?.disabled]).toEqual(['Compacting…', true]);
    expect(root.querySelector('[data-testid="panel-compact-problem"]')).toBeNull();
    give({ before_tokens: 49_920, after_tokens: 8_000 });
    await new Promise((resolve) => setTimeout(resolve));
    await fixture.whenStable();
    expect([button()?.textContent.trim(), button()?.disabled]).toEqual(['Compact now', false]);
  });

  it('shows only what the thread has', async () => {
    const fake = new FakeSession().answer('thread/get', {
      thread: thread('thread-1', 'Look around', {
        workspace: { ...WORKTREE, opened: { kind: 'folder', path: '/work/glave' } },
      }),
    });
    const { root } = await render(fake, []);
    expect(texts(root, 'panel-where-part')).toEqual([
      'Another computer',
      'glave · /worktrees/glave-0123',
    ]);
    for (const section of [
      'panel-plan',
      'panel-context',
      'panel-sign-in',
      'panel-changes',
      'panel-running',
    ]) {
      expect(root.querySelector(`[data-testid="${section}"]`)).toBeNull();
    }
    await expectAccessible(root);
  });
});
