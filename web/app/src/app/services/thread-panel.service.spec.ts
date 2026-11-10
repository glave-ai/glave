import { TestBed } from '@angular/core/testing';
import { ApiError, ErrorCode, type Opened, type WorkspaceBound } from '@glave/client';
import { fakeMedia } from '@glave/glaze/testing/media';

import { FakeSession, provideFakeSession } from '../testing/fake-session';
import { command, event, thread } from '../testing/fixtures';
import { ChangedFilesService } from './changed-files.service';
import { ChangesService } from './changes.service';
import { CommandsService } from './commands.service';
import { FirstTurnService } from './first-turn.service';
import { PanelService } from './panel.service';
import { PlanService } from './plan.service';
import { ThreadPanelService } from './thread-panel.service';
import { ThreadService } from './thread.service';

function workspace(opened: Opened, kind: 'local' | 'person' = 'person'): WorkspaceBound {
  return { workspace: 'w1', computer: { id: 'c1', kind }, opened, root: '/w' };
}

/** The panel's service on `fake`, its thread holding `events`, not yet opened. */
function provide(
  fake: FakeSession,
  events: readonly unknown[] = [],
  narrow = false,
): ThreadPanelService {
  fake.answer('thread/events', { events });
  fakeMedia(narrow);
  TestBed.configureTestingModule({
    providers: [
      ...provideFakeSession(fake),
      ThreadService,
      CommandsService,
      ChangedFilesService,
      PlanService,
      ThreadPanelService,
    ],
  });
  return TestBed.inject(ThreadPanelService);
}

/** Runs the effects, lets the calls they made answer, and runs the effects of those answers. */
async function settle(): Promise<void> {
  TestBed.tick();
  await new Promise((resolve) => setTimeout(resolve));
  TestBed.tick();
}

/** The panel's service on `fake`, its thread holding `events`, once opened. */
async function make(
  fake: FakeSession,
  events: readonly unknown[] = [],
  narrow = false,
): Promise<ThreadPanelService> {
  const panel = provide(fake, events, narrow);
  await TestBed.inject(ThreadService).open('thread-1');
  await settle();
  return panel;
}

const EDITED = [
  event(1, 'tool.call.requested', { call_id: 'c1', tool: 'edit', arguments: { json: {} } }),
  event(2, 'tool.call.completed', { call_id: 'c1', is_error: false, output: [] }),
];

describe('ThreadPanelService', () => {
  beforeEach(() => {
    localStorage.clear();
  });

  afterEach(() => {
    vi.unstubAllGlobals();
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
        cost: { nanos: 4_200_000, currency: 'USD' },
        context: { percent: 3, tokens: 3840, window: 128000 },
      }),
    });
    const panel = await make(fake);
    expect([panel.shown(), panel.visible()]).toEqual([false, false]);
  });

  it('knows whether there is a panel once the thread is, or a first message was sent here', async () => {
    const fake = new FakeSession();
    const summary = fake.held('thread/get');
    const panel = provide(fake);
    const opening = TestBed.inject(ThreadService).open('thread-1');
    await settle();
    expect(panel.decided()).toBe(false);
    summary({ thread: thread('thread-1', 'Fix it') });
    await opening;
    expect(panel.decided()).toBe(true);
    // One the server could not read is known to say so; a first message, to have none yet.
    TestBed.resetTestingModule();
    const refused = new ApiError({ code: ErrorCode.INTERNAL, message: 'the server failed' });
    const failed = provide(new FakeSession().answer('thread/get', refused));
    await TestBed.inject(ThreadService).open('thread-1');
    expect(failed.decided()).toBe(true);
    TestBed.resetTestingModule();
    const quiet = new FakeSession();
    quiet.held('thread/get');
    const sent = provide(quiet);
    TestBed.inject(FirstTurnService).start({ thread: 'thread-1', prompt: 'Hi' });
    void TestBed.inject(ThreadService).open('thread-1');
    // Read at once too: it has nothing to wait for yet.
    expect([sent.decided(), sent.read()]).toEqual([true, true]);
  });

  it('is read once its history is, whatever else is unanswered, and stays so in the thread', async () => {
    const fake = new FakeSession();
    // What it changed, its plan, its commands and its sign-ins are never answered.
    for (const method of [
      'thread/changes',
      'thread/plan',
      'command/list',
      'connector/list',
    ] as const) {
      fake.held(method);
    }
    const panel = await make(fake, [
      ...EDITED,
      event(3, 'command.started', { command: 'c1', script: 'npm run dev' }),
    ]);
    expect(panel.read()).toBe(true);
    // Opened again, in another language, it stays read while it is read again.
    const threads = TestBed.inject(ThreadService);
    const again = threads.open('thread-1');
    TestBed.tick();
    expect(panel.read()).toBe(true);
    await again;
    // Another thread is read afresh.
    void threads.open('thread-2');
    TestBed.tick();
    expect(panel.read()).toBe(false);
  });

  it('closes its sheet once it has nothing left, so it never comes back over the page', async () => {
    const fake = new FakeSession()
      .answer('command/list', {
        commands: [command('c1', 'npm run dev')],
        previewed: false,
      })
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

  it("counts the apps its project's threads use that need signing in again", async () => {
    const connector = (id: string, status: 'connected' | 'sign_in_again'): object => ({
      id,
      name: id,
      status,
      categories: [],
      tags: [],
      company: id,
      description: '',
      docs: '',
      url: `https://${id}.example`,
      returns: 'text',
      sign_in: 'dcr',
    });
    let listed = [connector('linear', 'sign_in_again'), connector('notion', 'connected')];
    const fake = new FakeSession().answer('connector/list', () => ({
      connectors: listed,
      categories: [],
      companies: [],
      tags: [],
    }));
    const panel = await make(fake);
    expect(panel.signInAgain()).toBe(1);
    expect(fake.paramsOf('connector/list')).toEqual([{ used_in: 'project-1' }]);
    // Read again as the person's connections change.
    listed = [connector('linear', 'connected')];
    TestBed.inject(ChangesService).tell('connections');
    await settle();
    expect(panel.signInAgain()).toBe(0);
    // A list that cannot be read says nothing.
    TestBed.resetTestingModule();
    const unread = await make(
      new FakeSession().answer(
        'connector/list',
        new ApiError({ code: ErrorCode.INTERNAL, message: 'it failed' }),
      ),
    );
    expect(unread.signInAgain()).toBe(0);
  });

  it('compacts its context on request, and shows a refusal in the server’s words', async () => {
    const fake = new FakeSession();
    const give = fake.held('thread/compact');
    const panel = await make(fake);
    const compacted = panel.compact();
    expect(panel.compacting()).toBe(true);
    // Once is enough while it runs.
    void panel.compact();
    expect(fake.paramsOf('thread/compact')).toEqual([{ thread: 'thread-1' }]);
    give({ before_tokens: 90_000, after_tokens: 12_000 });
    await compacted;
    expect([panel.compacting(), panel.compactProblem()]).toEqual([false, null]);
    fake.answer(
      'thread/compact',
      new ApiError({ code: ErrorCode.INVALID_PARAMS, message: 'a turn is running in it' }),
    );
    await panel.compact();
    expect([panel.compacting(), panel.compactProblem()]).toEqual([
      false,
      'A turn is running in it.',
    ]);
    // Another thread starts afresh.
    void TestBed.inject(ThreadService).open('thread-2');
    TestBed.tick();
    expect(panel.compactProblem()).toBeNull();
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
  });

  it('shows the commands it keeps running, and a turn’s changes in full', async () => {
    const fake = new FakeSession().answer('command/list', {
      commands: [command('c1', 'npm run dev')],
      previewed: false,
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
    expect([panel.reviewing(), panel.reviewedFiles()]).toEqual([false, []]);
    panel.showChanges(turn, null);
    expect([panel.reviewing(), panel.reviewed()]).toEqual([true, null]);
    // Another thread starts afresh.
    panel.reset();
    expect(panel.reviewing()).toBe(false);
  });

  it('shows the first preview until the person chooses another, and asks for its tab', async () => {
    const at = (port: number): string => `http://127.0.0.1:${String(port)}/`;
    const dev = { command: 'c1', port: 5173, url: at(5173), framed: true, unframed: null };
    const docs = { command: 'c2', port: 3000, url: at(3000), framed: true, unframed: null };
    const fake = new FakeSession().answer('command/list', {
      commands: [
        command('c1', 'npm run dev', { ports: [{ port: 5173, url: at(5173), framed: true }] }),
        command('c2', 'npm run docs', { ports: [{ port: 3000, url: at(3000), framed: true }] }),
      ],
      previewed: true,
    });
    const panel = await make(fake, [
      event(1, 'command.started', { command: 'c1', script: 'npm run dev' }),
    ]);
    expect([panel.previews(), panel.preview()]).toEqual([[dev, docs], dev]);
    panel.choosePreview('c2:3000');
    expect(panel.preview()).toEqual(docs);
    const asked = panel.previewAsked();
    panel.showPreview(dev);
    expect([panel.preview(), panel.previewAsked()]).toEqual([dev, asked + 1]);
    // A choice that serves no more falls back to the first.
    panel.choosePreview('c9:1');
    expect(panel.preview()).toEqual(dev);
  });
});
