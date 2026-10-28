import { BreakpointObserver } from '@angular/cdk/layout';
import { TestBed } from '@angular/core/testing';
import type { Opened, WorkspaceBound } from '@glave/client';
import { of } from 'rxjs';

import { FakeSession, provideFakeSession } from '../testing/fake-session';
import { command, event, thread } from '../testing/fixtures';
import { ChangedFilesService } from './changed-files.service';
import { CommandsService } from './commands.service';
import { PanelService } from './panel.service';
import { PlanService } from './plan.service';
import { SourcesService } from './sources.service';
import { ThreadPanelService } from './thread-panel.service';
import { ThreadService } from './thread.service';

function workspace(opened: Opened, kind: 'local' | 'worker' = 'worker'): WorkspaceBound {
  return { workspace: 'w1', computer: { id: 'c1', kind }, opened, root: '/w' };
}

const READ = [
  event(1, 'message.added', {
    role: 'user',
    parts: [
      { type: 'text', text: 'What changed in Tokio?' },
      { type: 'file', name: 'notes.md', bytes: 120 },
    ],
  }),
  event(2, 'tool.call.requested', {
    call_id: 'c1',
    tool: 'web_fetch',
    arguments: { json: { url: 'https://tokio.rs/blog' } },
  }),
  event(3, 'message.added', { role: 'assistant', parts: [{ type: 'text', text: 'Much.' }] }),
  event(4, 'turn.ended', { outcome: 'completed' }),
];

async function make(
  fake: FakeSession,
  events: readonly unknown[] = [],
  narrow = false,
): Promise<ThreadPanelService> {
  fake.answer('thread/events', { events });
  TestBed.configureTestingModule({
    providers: [
      { provide: BreakpointObserver, useValue: { observe: () => of({ matches: narrow }) } },
      ...provideFakeSession(fake),
      ThreadService,
      CommandsService,
      SourcesService,
      ChangedFilesService,
      PlanService,
      ThreadPanelService,
    ],
  });
  const panel = TestBed.inject(ThreadPanelService);
  await TestBed.inject(ThreadService).open('thread-1');
  TestBed.tick();
  await new Promise((resolve) => setTimeout(resolve));
  TestBed.tick();
  return panel;
}

describe('ThreadPanelService', () => {
  beforeEach(() => {
    localStorage.clear();
  });

  it('works in a folder the person chose, as its summary says; a chat’s scratch one is none', async () => {
    const at = async (opened: Opened): Promise<WorkspaceBound | null> => {
      TestBed.resetTestingModule();
      const fake = new FakeSession().answer('thread/get', {
        thread: thread('thread-1', 'Fix it', { workspace: workspace(opened) }),
      });
      return (await make(fake)).workspace();
    };
    const worktree = { kind: 'worktree', repository: '/work/glave', branch: 'glave/0123' } as const;
    expect((await at(worktree))?.opened).toEqual(worktree);
    expect((await at({ kind: 'folder', path: '/work/glave' }))?.root).toBe('/w');
    expect(await at({ kind: 'scratch', path: '/tmp/glave-1' })).toBeNull();
  });

  it('shows nothing for a plain chat, however much it cost', async () => {
    const fake = new FakeSession().answer('thread/get', {
      thread: thread('thread-1', 'Dinner ideas', {
        workspace: workspace({ kind: 'scratch', path: '/s' }, 'local'),
        cost: { micros: 4200, currency: 'USD' },
        context: { percent: 3, tokens: 3840, window: 128000 },
      }),
    });
    const panel = await make(fake);
    expect([panel.shown(), panel.visible()]).toEqual([false, false]);
  });

  it('closes its sheet once it has nothing left, so it never comes back over the page', async () => {
    const fake = new FakeSession()
      .answer('command/list', { commands: [command('c1', 'npm run dev')] })
      .answer('command/stop', {});
    const panel = await make(
      fake,
      [event(1, 'command.started', { command: 'c1', script: 'npm run dev' })],
      true,
    );
    const panels = TestBed.inject(PanelService);
    panels.setShowing(true);
    expect([panel.shown(), panels.showing()]).toEqual([true, true]);
    await TestBed.inject(CommandsService).stop('c1');
    TestBed.tick();
    expect([panel.shown(), panels.showing()]).toEqual([false, false]);
  });

  it('lists the pages its replies drew on once each, and the files the person attached', async () => {
    const page = { url: 'https://tokio.rs/blog', title: 'Tokio blog' };
    const fake = new FakeSession().answer('thread/sources', {
      sources: [
        { reply: 3, sentence: 'Sources: 1 page', pages: [page] },
        { reply: 7, sentence: 'Sources: 2 pages', pages: [page, { url: 'not a url' }] },
      ],
    });
    const panel = await make(fake, READ);
    expect(panel.pages().map((each) => [each.name, each.host])).toEqual([
      ['Tokio blog', 'tokio.rs'],
      ['not a url', 'not a url'],
    ]);
    expect(panel.attached().map((each) => each.name)).toEqual(['notes.md']);
    expect(panel.shown()).toBe(true);
  });

  it('shows the plan the server reads, once the thread has one', async () => {
    const plan = {
      proposed: false,
      sentence: 'Plan: 0 of 1 step done',
      steps: [{ step: 'Run the tests', status: 'pending' }],
    } as const;
    const fake = new FakeSession().answer('thread/plan', { plan });
    const panel = await make(fake, [
      event(1, 'plan.updated', { steps: plan.steps, call_id: 'c1' }),
    ]);
    expect(panel.plan()).toEqual(plan);
    expect(panel.shown()).toBe(true);
  });

  it('shows the files the server says it changed, and what they leave out', async () => {
    const files = [{ path: 'src/main.rs', changes: [], added: 1, removed: 0 }];
    const note = 'Changes made by commands or other tools are not listed.';
    const fake = new FakeSession().answer('thread/changes', {
      changes: { files, turns: [], note },
    });
    const panel = await make(fake, [
      event(1, 'tool.call.requested', { call_id: 'c1', tool: 'edit', arguments: { json: {} } }),
      event(2, 'tool.call.completed', { call_id: 'c1', is_error: false, output: [] }),
    ]);
    expect([panel.files(), panel.note(), panel.shown()]).toEqual([files, note, true]);
    expect(panel.reviewedFiles()).toBe(files);
  });

  it('shows the commands it keeps running, and a turn’s or the thread’s changes in full', async () => {
    const fake = new FakeSession().answer('command/list', {
      commands: [command('c1', 'npm run dev')],
    });
    const panel = await make(fake, [
      event(1, 'command.started', { command: 'c1', script: 'npm run dev' }),
    ]);
    const panels = TestBed.inject(PanelService);
    expect([panel.running(), panel.shown(), panel.visible()]).toEqual([1, true, true]);
    panels.setShowing(false);
    expect(panel.visible()).toBe(false);
    expect([panel.reviewing(), panel.reviewed()]).toEqual([false, null]);
    const turn = [{ path: 'src/main.rs', changes: [], added: 1, removed: 0 }];
    panel.showChanges(turn, 'src/main.rs');
    expect([panel.reviewing(), panel.reviewedFiles(), panel.reviewed()]).toEqual([
      true,
      turn,
      'src/main.rs',
    ]);
    panel.setReviewing(true);
    expect(panel.reviewed()).toBe('src/main.rs');
    panel.setReviewing(false);
    expect(panel.reviewing()).toBe(false);
    panel.showChanges(null, null);
    expect([panel.reviewing(), panel.reviewedFiles(), panel.reviewed()]).toEqual([true, [], null]);
    // Another thread starts afresh.
    panel.reset();
    expect(panel.reviewing()).toBe(false);
  });
});
