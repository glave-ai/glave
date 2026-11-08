import { TestBed } from '@angular/core/testing';
import type { WorkspaceBound } from '@glave/client';
import { expectAccessible } from '@glave/glaze/testing';
import { fakeMedia } from '@glave/glaze/testing/media';

import { PanelService } from '../../services/panel.service';
import { ProjectsService } from '../../services/projects.service';
import { ServerService } from '../../services/server.service';
import { ThreadService } from '../../services/thread.service';
import { FakeSession } from '../../testing/fake-session';
import { command, event, thread } from '../../testing/fixtures';
import { drawn, openForPanel } from '../../testing/thread-panel';
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
  fakeMedia(narrow);
  await openForPanel(fake, events);
  const fixture = TestBed.createComponent(ThreadPanelComponent);
  await drawn(fixture);
  return { root: fixture.nativeElement as HTMLElement, fixture };
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

  it('has a Terminal tab between Overview and Changes where the person may open terminals', async () => {
    const here = { ...FOLDER, computer: { id: 'c1', kind: 'local' } } as const;
    const fake = worked()
      .answer('thread/get', { thread: thread('thread-1', 'Add --verbose', { workspace: here }) })
      .answer('server/discover', { terminals: true })
      .answer('terminal/list', { terminals: [] })
      .answer('terminal/open', { terminal: { id: 't1', folder: '/work/glave' } });
    const { root, fixture } = await render(fake, WORKED);
    expect(tabs(root).map((tab) => tab.textContent.trim())).toEqual(['Overview', 'Changes']);
    await TestBed.inject(ServerService).load();
    await drawn(fixture);
    expect(tabs(root).map((tab) => tab.textContent.trim())).toEqual([
      'Overview',
      'Terminal',
      'Changes',
    ]);
    tabs(root)[1]?.click();
    await drawn(fixture);
    expect(root.querySelector('app-thread-overview')).toBeNull();
    expect(root.querySelector('app-terminal-tab')).not.toBeNull();
    await expectAccessible(root);
  });

  it('has no Changes tab before the thread changed files, and Overview again in each thread', async () => {
    const fake = worked();
    const give = fake.held('thread/changes');
    const { root, fixture } = await render(fake, WORKED);
    expect(tabs(root).map((tab) => tab.textContent.trim())).toEqual(['Overview']);
    give({ changes: { files: FILES, turns: [], note: '' } });
    await drawn(fixture);
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
    await drawn(fixture);
    expect(root.querySelector('[data-testid="command-stop"]')).toBeNull();
    expect(document.activeElement).toBe(root.querySelector('[data-testid="panel-heading"]'));
  });

  it('gives it to its heading too when Compact now takes the context away, never on a tab’s move', async () => {
    const fake = worked().answer('thread/get', {
      thread: thread('thread-1', 'Add --verbose', {
        workspace: FOLDER,
        context: { percent: 82, tokens: 104_960, window: 128_000 },
      }),
    });
    const { root, fixture } = await render(fake, WORKED);
    const compact = root.querySelector<HTMLButtonElement>('[data-testid="panel-compact"]');
    compact?.focus();
    compact?.click();
    await drawn(fixture);
    // Compacted, the thread says nothing of its context until its next call.
    fake.answer('thread/list', {
      threads: [
        thread('thread-1', 'Add --verbose', {
          workspace: FOLDER,
          updated_at: '2026-10-11T10:30:00.000Z',
        }),
      ],
    });
    await TestBed.inject(ProjectsService).refresh();
    await drawn(fixture);
    expect(root.querySelector('[data-testid="panel-compact"]')).toBeNull();
    const heading = root.querySelector('[data-testid="panel-heading"]');
    expect(document.activeElement).toBe(heading);
    // Moving to another tab, as a press on it does in Safari, which focuses no button, leaves
    // the keyboard's place where it was.
    root.querySelector<HTMLButtonElement>('[data-testid="command-stop"]')?.focus();
    tabs(root)[1]?.click();
    await drawn(fixture);
    expect(document.activeElement).not.toBe(heading);
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
