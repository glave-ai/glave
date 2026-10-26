import { TestBed } from '@angular/core/testing';
import type { Opened, WorkspaceBound } from '@glave/client';

import { FakeSession, provideFakeSession } from '../testing/fake-session';
import { command, event, thread } from '../testing/fixtures';
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
): Promise<ThreadPanelService> {
  fake.answer('thread/events', { events });
  TestBed.configureTestingModule({
    providers: [
      ...provideFakeSession(fake),
      ThreadService,
      CommandsService,
      SourcesService,
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

  it('says where a thread works as its summary says: the folder, a branch where it has one', async () => {
    const at = async (opened: Opened): Promise<unknown> => {
      TestBed.resetTestingModule();
      const fake = new FakeSession().answer('thread/get', {
        thread: thread('thread-1', 'Fix it', { workspace: workspace(opened), permissions: 'ask' }),
      });
      return (await make(fake)).where();
    };
    expect(await at({ kind: 'folder', path: '/work/glave' })).toEqual({
      computer: 'Another computer',
      folder: 'glave',
      path: '/work/glave',
      branch: null,
      mode: 'ask',
    });
    expect(
      await at({ kind: 'worktree', repository: '/work/glave', branch: 'glave/0123456789ab' }),
    ).toMatchObject({ folder: 'glave', branch: 'glave/0123456789ab' });
    expect(
      await at({ kind: 'clone', url: 'https://github.com/acme/app.git', at: 'main' }),
    ).toMatchObject({ folder: 'app', path: 'https://github.com/acme/app.git', branch: 'main' });
    // A chat's scratch folder is nowhere the person chose.
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
    expect(panel.spent()).toEqual({
      context: { percent: 3, tokens: 3840, window: 128000 },
      cost: { micros: 4200, currency: 'USD' },
      estimated: false,
      turns: 1,
    });
    expect(panel.shown()).toBe(false);
    // Open as the person left it, it still shows nothing.
    expect([TestBed.inject(PanelService).open(), panel.visible()]).toEqual([true, false]);
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

  it('shows the commands it keeps running, and its changes in full on asking', async () => {
    const fake = new FakeSession().answer('command/list', {
      commands: [command('c1', 'npm run dev')],
    });
    const panel = await make(fake, [
      event(1, 'command.started', { command: 'c1', script: 'npm run dev' }),
    ]);
    expect([panel.running(), panel.shown(), panel.visible()]).toEqual([1, true, true]);
    TestBed.inject(PanelService).open.set(false);
    expect(panel.visible()).toBe(false);
    expect([panel.reviewing(), panel.reviewed()]).toEqual([false, null]);
    panel.showChanges('src/main.rs');
    expect([panel.reviewing(), panel.reviewed()]).toEqual([true, 'src/main.rs']);
    panel.setReviewing(true);
    expect(panel.reviewed()).toBe('src/main.rs');
    panel.setReviewing(false);
    expect(panel.reviewing()).toBe(false);
    panel.showChanges(null);
    expect([panel.reviewing(), panel.reviewed()]).toEqual([true, null]);
  });
});
