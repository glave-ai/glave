import { TestBed } from '@angular/core/testing';
import type { WorkspaceBound } from '@glave/client';
import { expectAccessible } from '@glave/glaze/testing';
import { fakeMedia } from '@glave/glaze/testing/media';

import { ChangedFilesService } from '../../services/changed-files.service';
import { CommandsService } from '../../services/commands.service';
import { PanelService } from '../../services/panel.service';
import { PlanService } from '../../services/plan.service';
import { ProjectsService } from '../../services/projects.service';
import { SourcesService } from '../../services/sources.service';
import { ThreadPanelService } from '../../services/thread-panel.service';
import { ThreadService } from '../../services/thread.service';
import { FakeSession, provideFakeSession } from '../../testing/fake-session';
import { command, event, record, thread } from '../../testing/fixtures';
import { ThreadPanelComponent } from './thread-panel.component';

/** A turn that read a page, changed two files, keeps a dev server running and has a plan. */
const WORKED = [
  event(1, 'message.added', {
    role: 'user',
    parts: [
      { type: 'text', text: 'Add a --verbose flag' },
      { type: 'file', name: 'terminal-output.png', blob: { bytes: 2048 } },
    ],
  }),
  event(2, 'tool.call.requested', {
    call_id: 'c1',
    tool: 'web_fetch',
    arguments: { json: { url: 'https://docs.rs/clap' } },
  }),
  event(3, 'tool.call.completed', { call_id: 'c1', is_error: false, output: [] }),
  event(4, 'tool.call.requested', {
    call_id: 'c2',
    tool: 'edit',
    arguments: { json: { path: 'src/args.rs', old_string: 'x', new_string: 'a\nb\nc' } },
  }),
  event(5, 'tool.call.completed', { call_id: 'c2', is_error: false, output: [] }),
  event(6, 'tool.call.requested', {
    call_id: 'c3',
    tool: 'write',
    arguments: { json: { path: 'src/main.rs', content: 'fn main() {}\n' } },
  }),
  event(7, 'tool.call.completed', { call_id: 'c3', is_error: false, output: [] }),
  event(8, 'command.started', { command: 'd1', script: 'npm run dev' }),
  event(9, 'plan.updated', { steps: [], call_id: 'c4' }),
  event(10, 'message.added', { role: 'assistant', parts: [{ type: 'text', text: 'Done.' }] }),
  event(11, 'turn.ended', { outcome: 'completed' }),
];

const WORKTREE: WorkspaceBound = {
  workspace: 'w1',
  computer: { id: 'c1', kind: 'worker' },
  opened: { kind: 'worktree', repository: '/work/glave', branch: 'glave/0123456789ab' },
  root: '/worktrees/glave-0123',
};

async function render(
  fake: FakeSession,
  events: readonly unknown[],
  narrow = false,
  read = true,
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
      ThreadService,
      CommandsService,
      SourcesService,
      ChangedFilesService,
      PlanService,
      ThreadPanelService,
    ],
  });
  if (read) {
    await TestBed.inject(ProjectsService).refresh();
  }
  await TestBed.inject(ThreadService).open('thread-1');
  const fixture = TestBed.createComponent(ThreadPanelComponent);
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

function worked(workspace: WorkspaceBound = WORKTREE): FakeSession {
  return new FakeSession()
    .answer('thread/get', {
      thread: thread('thread-1', 'Add a --verbose flag', {
        workspace,
        turns: 3,
        cost: { micros: 12_000, currency: 'USD' },
        context: { percent: 39, tokens: 49_920, window: 128_000 },
      }),
    })
    .answer('command/list', { commands: [command('d1', 'npm run dev')] })
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
    .answer('thread/sources', {
      sources: [
        {
          reply: 10,
          sentence: 'Sources: 1 page',
          pages: [{ url: 'https://docs.rs/clap', title: 'clap: Arg::action' }],
        },
      ],
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

describe('ThreadPanelComponent', () => {
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it("keeps its facts from sight until the thread's project is known", async () => {
    const { root, fixture } = await render(worked(), WORKED, false, false);
    const facts = root.querySelector('[data-testid="panel-facts"]');
    expect(facts?.classList.contains('reading')).toBe(true);
    await TestBed.inject(ProjectsService).refresh();
    await fixture.whenStable();
    expect(facts?.classList.contains('reading')).toBe(false);
    expect(texts(root, 'panel-where-part')).toContain('glave');
  });

  it('shows its facts, with no name, when the lists hold no such project', async () => {
    const fake = worked();
    const { root, fixture } = await render(fake, WORKED, false, false);
    fake.answer('project/list', { projects: [], gone: [] });
    await TestBed.inject(ProjectsService).refresh();
    await fixture.whenStable();
    const facts = root.querySelector('[data-testid="panel-facts"]');
    expect(facts?.classList.contains('reading')).toBe(false);
    expect(texts(root, 'panel-where-part')).not.toContain('glave');
  });

  it('shows its facts, with no name, when the lists cannot be read', async () => {
    const fake = worked();
    const { root, fixture } = await render(fake, WORKED, false, false);
    fake.answer('project/list', () => {
      throw new Error('it could not be read');
    });
    await TestBed.inject(ProjectsService).refresh();
    await fixture.whenStable();
    const facts = root.querySelector('[data-testid="panel-facts"]');
    expect(facts?.classList.contains('reading')).toBe(false);
    expect(texts(root, 'panel-where-part')).not.toContain('glave');
  });

  it('shows where the thread works and each section that has something', async () => {
    const { root } = await render(worked(), WORKED);
    expect(texts(root, 'panel-where-part')).toEqual([
      'Another computer',
      'glave',
      'glave/0123456789ab',
      'Auto',
    ]);
    const marks = root.querySelectorAll('[data-testid="panel-where-part"] glz-icon');
    expect(Array.from(marks).map((icon) => icon.getAttribute('aria-label'))).toEqual([
      'Computer',
      'Folder',
      'Branch',
      'Mode',
    ]);
    // The project's own name; where its tools act in full, a press away, by keyboard or touch.
    const folder = root.querySelector<HTMLDetailsElement>('[data-testid="panel-folder"]');
    expect(folder?.open).toBe(false);
    expect(texts(root, 'panel-path')).toEqual(['/worktrees/glave-0123']);
    expect(texts(root, 'panel-plan-step')).toEqual(['Read how flags are parsed', 'Add --verbose']);
    expect(root.querySelector('[data-testid="panel-plan"]')?.textContent).toContain(
      'Plan: 1 of 2 steps done',
    );
    expect(texts(root, 'panel-changes-file')).toEqual([
      'src/args.rs+3−13 lines added, 1 removed',
      'src/main.rs+1−01 line added, 0 removed',
    ]);
    expect(texts(root, 'command-summary')[0]).toContain('Still running');
    expect(texts(root, 'command-text')).toEqual(['npm run dev']);
    const page = root.querySelector<HTMLAnchorElement>('[data-testid="panel-page"]');
    expect(page?.href).toBe('https://docs.rs/clap');
    expect(page?.target).toBe('_blank');
    expect(texts(root, 'panel-page')).toEqual(['clap: Arg::action docs.rs (opens in a new tab)']);
    expect(texts(root, 'panel-attached')).toEqual(['terminal-output.png you attached']);
    expect(texts(root, 'panel-context')).toEqual(['39% used']);
    const meter = root.querySelector<HTMLMeterElement>('meter[data-testid="panel-meter"]');
    expect([meter?.value, meter?.max, meter?.getAttribute('aria-valuetext')]).toEqual([
      39,
      100,
      '39% used',
    ]);
    expect(texts(root, 'panel-cost')).toEqual(['$0.01 · 3 turns']);
    await expectAccessible(root);
  });

  it('keeps the room of the files the thread changed, unseen, until they are read', async () => {
    const fake = worked();
    const give = fake.held('thread/changes');
    const ended = event(11, 'turn.ended', {
      outcome: 'completed',
      changed: ['src/args.rs', 'src/main.rs'],
    });
    const { root, fixture } = await render(fake, [...WORKED.slice(0, -1), ended]);
    const kept = root.querySelector('app-turn-changes.room');
    expect(kept?.querySelectorAll('[data-testid="panel-changes-room-file"]')).toHaveLength(2);
    expect(getComputedStyle(kept ?? root).visibility).toBe('hidden');
    expect(root.querySelector('[data-testid="panel-changes"]')).toBeNull();
    await expectAccessible(root);
    give({ changes: { files: [], turns: [], note: '' } });
    await new Promise((resolve) => setTimeout(resolve));
    await fixture.whenStable();
    expect(root.querySelector('app-turn-changes.room')).toBeNull();
  });

  it('heads itself one level up from its sections, heard alone beside the page', async () => {
    const { root } = await render(worked(), WORKED);
    const heading = root.querySelector('h2');
    expect(heading?.textContent.trim()).toBe('This thread');
    expect(heading?.classList).toContain('glz-visually-hidden');
    expect(Array.from(root.querySelectorAll('h3')).map((each) => each.textContent.trim())).toEqual([
      'Plan',
      'Changes',
      'Running 1',
      'Sources and files',
      'Context and cost',
    ]);
    const close = root.querySelector<HTMLElement>('[data-testid="panel-close"]');
    expect(close === null ? null : getComputedStyle(close).display).toBe('none');
  });

  it('names the point a clone is at as no branch, and opens changes in full', async () => {
    const clone: WorkspaceBound = {
      ...WORKTREE,
      opened: { kind: 'clone', url: 'https://github.com/acme/app.git', at: 'v1.2.0' },
    };
    const { root } = await render(worked(clone), WORKED);
    const parts = Array.from(root.querySelectorAll('[data-testid="panel-where-part"] glz-icon'));
    expect(parts.map((icon) => icon.getAttribute('aria-label'))).toContain('Checked out at');
    expect(parts.map((icon) => icon.getAttribute('aria-label'))).not.toContain('Branch');
    const panel = TestBed.inject(ThreadPanelService);
    root.querySelectorAll<HTMLButtonElement>('[data-testid="panel-changes-file"]')[1]?.click();
    expect(panel.reviewed()).toBe('src/main.rs');
    root.querySelector<HTMLButtonElement>('[data-testid="panel-review"]')?.click();
    expect([panel.reviewing(), panel.reviewed()]).toEqual([true, null]);
  });

  it('shows only what the thread has, and says a cost not reported so', async () => {
    const fake = new FakeSession().answer('thread/get', {
      thread: thread('thread-1', 'Look around', {
        workspace: { ...WORKTREE, opened: { kind: 'folder', path: '/work/glave' } },
        estimated: true,
        cost: { micros: 1_250_000, currency: 'USD' },
      }),
    });
    const { root } = await render(fake, []);
    expect(texts(root, 'panel-where-part')).toEqual(['Another computer', 'glave', 'Auto']);
    for (const section of [
      'panel-plan',
      'panel-changes',
      'panel-running',
      'panel-sources',
      'panel-context',
    ]) {
      expect(root.querySelector(`[data-testid="${section}"]`)).toBeNull();
    }
    expect(texts(root, 'panel-cost')).toEqual(['$1.25 (estimated) · 1 turn']);
    TestBed.resetTestingModule();
    const unreported = await render(
      new FakeSession().answer('thread/get', { thread: thread('thread-1', 'x', { turns: 2 }) }),
      [],
    );
    expect(texts(unreported.root, 'panel-cost')).toEqual(['cost not reported · 2 turns']);
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
