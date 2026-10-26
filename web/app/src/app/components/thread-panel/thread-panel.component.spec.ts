import { BreakpointObserver } from '@angular/cdk/layout';
import { TestBed } from '@angular/core/testing';
import { expectAccessible } from '@glave/glaze/testing';
import { of } from 'rxjs';

import { CommandsService } from '../../services/commands.service';
import { PanelService } from '../../services/panel.service';
import { PlanService } from '../../services/plan.service';
import { SourcesService } from '../../services/sources.service';
import { ThreadPanelService } from '../../services/thread-panel.service';
import { ThreadService } from '../../services/thread.service';
import { FakeSession, provideFakeSession } from '../../testing/fake-session';
import { command, event, thread } from '../../testing/fixtures';
import { ThreadPanelComponent } from './thread-panel.component';

/** A turn that read a page, changed two files and keeps a dev server running. */
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

const WORKSPACE = {
  workspace: 'w1',
  computer: { id: 'c1', kind: 'worker' },
  opened: { kind: 'worktree', repository: '/work/glave', branch: 'glave/0123456789ab' },
  root: '/w',
} as const;

async function render(
  fake: FakeSession,
  events: readonly unknown[],
  narrow = false,
): Promise<HTMLElement> {
  localStorage.clear();
  fake.answer('thread/events', { events });
  TestBed.configureTestingModule({
    providers: [
      ...provideFakeSession(fake),
      ThreadService,
      CommandsService,
      SourcesService,
      PlanService,
      ThreadPanelService,
      { provide: BreakpointObserver, useValue: { observe: () => of({ matches: narrow }) } },
    ],
  });
  await TestBed.inject(ThreadService).open('thread-1');
  const fixture = TestBed.createComponent(ThreadPanelComponent);
  await fixture.whenStable();
  await new Promise((resolve) => setTimeout(resolve));
  await fixture.whenStable();
  return fixture.nativeElement as HTMLElement;
}

function texts(root: HTMLElement, testId: string): readonly string[] {
  return Array.from(root.querySelectorAll(`[data-testid="${testId}"]`)).map((each) =>
    each.textContent.replace(/\s+/gu, ' ').trim(),
  );
}

function worked(): FakeSession {
  return new FakeSession()
    .answer('thread/get', {
      thread: thread('thread-1', 'Add a --verbose flag', {
        workspace: WORKSPACE,
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
    });
}

describe('ThreadPanelComponent', () => {
  it('shows where the thread works and each section that has something', async () => {
    const root = await render(worked(), WORKED);
    expect(root.querySelector('[data-testid="panel-close"]')).toBeNull();
    expect(texts(root, 'panel-where-part')).toEqual([
      'Another computer',
      'glave',
      'glave/0123456789ab',
      'Auto',
    ]);
    const parts = root.querySelectorAll('[data-testid="panel-where-part"] glz-icon');
    expect(Array.from(parts).map((icon) => icon.getAttribute('aria-label'))).toEqual([
      'Computer',
      'Folder',
      'Branch',
      'Mode',
    ]);
    const plan = root.querySelector('[data-testid="panel-plan"]');
    expect(plan?.textContent).toContain('Plan: 1 of 2 steps done');
    expect(texts(root, 'panel-plan-step')).toEqual(['Read how flags are parsed', 'Add --verbose']);
    const changes = root.querySelector('[data-testid="panel-changes"]');
    expect(changes?.querySelector('h2')?.textContent).toBe('Changes');
    expect(changes?.textContent).toContain('2 files');
    expect(changes?.textContent).toContain('4 lines added, 1 removed');
    expect(texts(root, 'panel-file')).toEqual([
      'src/args.rs+3−13 lines added, 1 removed',
      'src/main.rs+1−01 line added, 0 removed',
    ]);
    expect(texts(root, 'command-text')).toEqual(['npm run dev']);
    const page = root.querySelector<HTMLAnchorElement>('[data-testid="panel-page"]');
    expect(page?.href).toBe('https://docs.rs/clap');
    expect(page?.target).toBe('_blank');
    expect(texts(root, 'panel-page')).toEqual(['clap: Arg::action docs.rs (opens in a new tab)']);
    expect(texts(root, 'panel-attached')).toEqual(['terminal-output.png you attached']);
    expect(texts(root, 'panel-context')).toEqual(['39% used']);
    const meter = root.querySelector('[data-testid="panel-meter"]');
    expect(meter?.getAttribute('aria-valuenow')).toBe('39');
    expect(meter?.getAttribute('aria-valuetext')).toBe('39% used');
    expect(root.style.getPropertyValue('--app-context-used')).toBe('39%');
    expect(texts(root, 'panel-cost')).toEqual(['$0.01 · 3 turns']);
    await expectAccessible(root);
  });

  it('opens a file’s changes in full, or every file’s', async () => {
    const root = await render(worked(), WORKED);
    const panel = TestBed.inject(ThreadPanelService);
    root.querySelectorAll<HTMLButtonElement>('[data-testid="panel-file"]')[1]?.click();
    expect(panel.reviewed()).toBe('src/main.rs');
    root.querySelector<HTMLButtonElement>('[data-testid="panel-review"]')?.click();
    expect([panel.reviewing(), panel.reviewed()]).toEqual([true, null]);
  });

  it('shows only what the thread has, and says a cost not reported so', async () => {
    const fake = new FakeSession().answer('thread/get', {
      thread: thread('thread-1', 'Look around', {
        workspace: { ...WORKSPACE, opened: { kind: 'folder', path: '/work/glave' } },
        estimated: true,
        cost: { micros: 1_250_000, currency: 'USD' },
      }),
    });
    const root = await render(fake, []);
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
    expect(texts(unreported, 'panel-cost')).toEqual(['cost not reported · 2 turns']);
  });

  it('as a sheet, has a heading and closes', async () => {
    const root = await render(worked(), WORKED, true);
    const panel = TestBed.inject(PanelService);
    panel.open.set(true);
    expect(root.querySelector('h2')?.textContent).toBe('This thread');
    root.querySelector<HTMLButtonElement>('[data-testid="panel-close"]')?.click();
    expect(panel.open()).toBe(false);
  });
});
