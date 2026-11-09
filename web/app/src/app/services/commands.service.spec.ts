import { TestBed } from '@angular/core/testing';
import { ApiError, ErrorCode, type Notification, type Port, type RawEvent } from '@glave/client';

import { LOOKS_AGAIN } from '../lib/previews';
import { FakeSession, provideFakeSession } from '../testing/fake-session';
import { command, event } from '../testing/fixtures';
import { TICK_MS } from './clock';
import { CommandsService } from './commands.service';
import { ThreadService } from './thread.service';
import { PanelService } from './panel.service';

const STARTED = event(1, 'command.started', { command: 'c1', script: 'npm run dev' });

function recorded(raw: RawEvent): Notification {
  return { jsonrpc: '2.0', method: 'thread/event', params: { thread: raw.thread, event: raw } };
}

function make(fake: FakeSession): {
  readonly commands: CommandsService;
  readonly thread: ThreadService;
  readonly panel: PanelService;
} {
  TestBed.configureTestingModule({
    providers: [...provideFakeSession(fake), ThreadService, CommandsService],
  });
  return {
    commands: TestBed.inject(CommandsService),
    thread: TestBed.inject(ThreadService),
    panel: TestBed.inject(PanelService),
  };
}

/** Runs the effects, lets the calls they made answer, and runs the effects of those answers. */
async function settle(): Promise<void> {
  TestBed.tick();
  await new Promise((resolve) => setTimeout(resolve));
  TestBed.tick();
}

/** A thread that started `npm run dev`, which listens at `ports` as it runs, previewed here. */
function devServer(ports: readonly Port[]): FakeSession {
  return new FakeSession().answer('thread/events', { events: [STARTED] }).answer('command/list', {
    commands: [command('c1', 'npm run dev', { ports })],
    previewed: true,
  });
}

/** `npm run dev`'s port 5173, previewed at its own address. */
const DEV: Port = { port: 5173, url: 'http://127.0.0.1:5173/', framed: true };

function ids(commands: CommandsService): readonly string[] {
  return commands.commands().map((each) => each.id);
}

describe('CommandsService', () => {
  beforeEach(() => {
    localStorage.clear();
  });

  it('reads what runs once the thread records a command, and as each starts or ends', async () => {
    const fake = new FakeSession()
      .answer('thread/events', { events: [] })
      .answer('command/list', {
        commands: [command('c1', 'npm run dev')],
        previewed: false,
      })
      .streams([
        recorded(STARTED),
        recorded(event(2, 'command.ended', { command: 'c0', ending: { kind: 'exited' } })),
      ]);
    const { commands, thread } = make(fake);
    await thread.open('thread-1');
    await settle();
    // A thread that never kept a command in the background has nothing to ask about.
    expect(fake.paramsOf('command/list')).toEqual([]);
    expect(commands.loaded()).toBe(false);
    await thread.send('Start the dev server');
    await settle();
    expect(fake.paramsOf('command/list')).toEqual([{ thread: 'thread-1' }]);
    expect(ids(commands)).toEqual(['c1']);
    expect(commands.count()).toBe(1);
    expect([commands.loaded(), commands.error()]).toEqual([true, null]);
  });

  it('reads what runs again each time the panel is shown again, once the thread kept one', async () => {
    const fake = new FakeSession().answer('thread/events', { events: [] });
    const { commands, thread, panel } = make(fake);
    await thread.open('thread-1');
    await settle();
    panel.setShowing(false);
    await settle();
    panel.setShowing(true);
    await settle();
    // Nothing kept in the background, nothing to ask about.
    expect(fake.paramsOf('command/list')).toHaveLength(0);
    fake.answer('thread/events', { events: [STARTED] });
    await thread.open('thread-2');
    await settle();
    expect(fake.paramsOf('command/list')).toEqual([{ thread: 'thread-2' }]);
    expect(commands.loaded()).toBe(true);
    panel.setShowing(false);
    await settle();
    expect(fake.paramsOf('command/list')).toHaveLength(1);
    panel.setShowing(true);
    await settle();
    expect(fake.paramsOf('command/list')).toHaveLength(2);
  });

  it('starts from nothing in another thread', async () => {
    const fake = new FakeSession()
      .answer('thread/events', (params) => ({
        events: (params as { thread: string }).thread === 'thread-1' ? [STARTED] : [],
      }))
      .answer('command/list', {
        commands: [command('c1', 'npm run dev')],
        previewed: false,
      });
    const { commands, thread } = make(fake);
    await thread.open('thread-1');
    await settle();
    expect(ids(commands)).toEqual(['c1']);
    await thread.open('thread-2');
    expect([ids(commands), commands.loaded()]).toEqual([[], false]);
  });

  it('shows a command stopping, then drops it; one that had already ended goes too', async () => {
    let answer: (value: object) => void = () => undefined;
    const fake = new FakeSession()
      .answer('thread/events', { events: [STARTED] })
      .answer('command/list', {
        commands: [command('c1', 'npm run dev'), command('c2', 'cargo watch')],
        previewed: false,
      })
      .answer(
        'command/stop',
        () =>
          new Promise((resolve) => {
            answer = resolve;
          }),
      );
    const { commands, thread } = make(fake);
    await thread.open('thread-1');
    await settle();
    const stopping = commands.stop('c1');
    expect([...commands.stopping()]).toEqual(['c1']);
    // A second press while it stops sends nothing more.
    await commands.stop('c1');
    expect(fake.paramsOf('command/stop')).toEqual([{ thread: 'thread-1', command: 'c1' }]);
    answer({});
    await stopping;
    expect([ids(commands), commands.stopping().size]).toEqual([['c2'], 0]);
    fake.answer(
      'command/stop',
      new ApiError({ code: ErrorCode.NO_COMMAND, message: 'that command is not running' }),
    );
    await commands.stop('c2');
    expect([ids(commands), commands.refusals().size]).toEqual([[], 0]);
  });

  it("keeps a command the server would not stop, with the server's words", async () => {
    const fake = new FakeSession()
      .answer('thread/events', { events: [STARTED] })
      .answer('command/list', {
        commands: [command('c1', 'npm run dev')],
        previewed: false,
      })
      .answer(
        'command/stop',
        new ApiError({ code: ErrorCode.INTERNAL, message: 'the command could not be stopped' }),
      );
    const { commands, thread } = make(fake);
    await thread.open('thread-1');
    await settle();
    await commands.stop('c1');
    expect(ids(commands)).toEqual(['c1']);
    expect(commands.refusals().get('c1')).toBe('The command could not be stopped.');
    fake.answer('command/stop', {});
    const again = commands.stop('c1');
    expect(commands.refusals().size).toBe(0);
    await again;
    expect(ids(commands)).toEqual([]);
  });

  it('drops a read the server answered after a stop it began before', async () => {
    let answer: (value: object) => void = () => undefined;
    const fake = new FakeSession()
      .answer('thread/events', { events: [STARTED] })
      .answer('command/list', {
        commands: [command('c1', 'npm run dev')],
        previewed: false,
      });
    const { commands, thread } = make(fake);
    await thread.open('thread-1');
    await settle();
    fake.answer(
      'command/list',
      () =>
        new Promise((resolve) => {
          answer = resolve;
        }),
    );
    const reading = commands.refresh();
    await commands.stop('c1');
    answer({ commands: [command('c1', 'npm run dev')], previewed: false });
    await reading;
    expect(ids(commands)).toEqual([]);
  });

  it('says why it could not read them', async () => {
    const fake = new FakeSession()
      .answer('thread/events', { events: [STARTED] })
      .answer(
        'command/list',
        new ApiError({ code: ErrorCode.INTERNAL, message: 'the computer did not answer' }),
      );
    const { commands, thread } = make(fake);
    await thread.open('thread-1');
    await settle();
    expect([commands.loaded(), commands.error()]).toEqual([true, 'The computer did not answer.']);
  });

  it('moves its clock on each second while the panel shows, and not after', async () => {
    vi.useFakeTimers({ toFake: ['Date', 'setInterval', 'clearInterval'] });
    try {
      vi.setSystemTime(Date.parse('2026-10-11T10:02:05.000Z'));
      const { commands, thread, panel } = make(devServer([]));
      panel.setShowing(true);
      await thread.open('thread-1');
      await settle();
      const shown = commands.now();
      expect(shown).toBe(Date.parse('2026-10-11T10:02:05.000Z'));
      vi.advanceTimersByTime(TICK_MS * 2);
      expect(commands.now()).toBe(shown + TICK_MS * 2);
      panel.setShowing(false);
      TestBed.tick();
      vi.advanceTimersByTime(TICK_MS * 5);
      expect(commands.now()).toBe(shown + TICK_MS * 2);
    } finally {
      vi.useRealTimers();
    }
  });

  it('reads the previews of what they serve from their ports, and drops a stopped command’s', async () => {
    const { commands, thread } = make(devServer([DEV]).answer('command/stop', {}));
    await thread.open('thread-1');
    await settle();
    expect(commands.previews()).toEqual([
      { command: 'c1', port: 5173, url: DEV.url, framed: true, unframed: null },
    ]);
    await commands.stop('c1');
    expect(commands.previews()).toEqual([]);
  });

  it('reads them again later and later while previewing and the panel shows, previews or not', async () => {
    vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout'] });
    try {
      // One port previewed already: another, as a second server beside it, may come later.
      const fake = devServer([DEV]);
      const { commands, thread, panel } = make(fake);
      panel.setShowing(true);
      await thread.open('thread-1');
      TestBed.tick();
      await vi.advanceTimersByTimeAsync(0);
      expect(fake.paramsOf('command/list')).toHaveLength(1);
      // Each read again waits longer than the one before, until there are no more.
      for (const [looked, wait] of LOOKS_AGAIN.entries()) {
        await vi.advanceTimersByTimeAsync(wait - 1);
        expect(fake.paramsOf('command/list')).toHaveLength(looked + 1);
        await vi.advanceTimersByTimeAsync(1);
        expect(fake.paramsOf('command/list')).toHaveLength(looked + 2);
      }
      await vi.advanceTimersByTimeAsync(600_000);
      expect(fake.paramsOf('command/list')).toHaveLength(LOOKS_AGAIN.length + 1);

      // Read again by hand, it looks again as before; not while the panel is hidden.
      panel.setShowing(false);
      TestBed.tick();
      await commands.refresh();
      await vi.advanceTimersByTimeAsync(600_000);
      expect(fake.paramsOf('command/list')).toHaveLength(LOOKS_AGAIN.length + 2);
    } finally {
      vi.useRealTimers();
    }
  });

  it('never reads them again for previews where the server previews nothing', async () => {
    vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout'] });
    try {
      const fake = new FakeSession()
        .answer('thread/events', { events: [STARTED] })
        .answer('command/list', {
          commands: [command('c1', 'npm run dev', { ports: [{ port: 5173, framed: false }] })],
          previewed: false,
        });
      const { commands, thread, panel } = make(fake);
      panel.setShowing(true);
      await thread.open('thread-1');
      TestBed.tick();
      await vi.advanceTimersByTimeAsync(600_000);
      expect(fake.paramsOf('command/list')).toHaveLength(1);
      expect(commands.previews()).toEqual([]);
    } finally {
      vi.useRealTimers();
    }
  });
});
