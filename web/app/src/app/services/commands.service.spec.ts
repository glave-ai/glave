import { TestBed } from '@angular/core/testing';
import { ApiError, ErrorCode, type Notification, type RawEvent } from '@glave/client';

import { FakeSession, provideFakeSession } from '../testing/fake-session';
import { command, event } from '../testing/fixtures';
import { COMMANDS_TICK_MS, CommandsService } from './commands.service';
import { ThreadService } from './thread.service';
import { ToolsPanelService } from './tools-panel.service';

const STARTED = event(1, 'command.started', { command: 'c1', script: 'npm run dev' });

function recorded(raw: RawEvent): Notification {
  return { jsonrpc: '2.0', method: 'thread/event', params: { thread: raw.thread, event: raw } };
}

function make(fake: FakeSession): {
  readonly commands: CommandsService;
  readonly thread: ThreadService;
  readonly panel: ToolsPanelService;
} {
  TestBed.configureTestingModule({
    providers: [...provideFakeSession(fake), ThreadService, CommandsService],
  });
  return {
    commands: TestBed.inject(CommandsService),
    thread: TestBed.inject(ThreadService),
    panel: TestBed.inject(ToolsPanelService),
  };
}

/** Runs the effects, lets the calls they made answer, and runs the effects of those answers. */
async function settle(): Promise<void> {
  TestBed.tick();
  await new Promise((resolve) => setTimeout(resolve));
  TestBed.tick();
}

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
      .answer('command/list', { commands: [command('c1', 'npm run dev')] })
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

  it('reads what runs when the tools panel opens on the commands', async () => {
    const fake = new FakeSession().answer('thread/events', { events: [] });
    const { commands, thread, panel } = make(fake);
    await thread.open('thread-1');
    panel.tab.set('commands');
    await settle();
    expect(fake.paramsOf('command/list')).toHaveLength(0);
    panel.open.set(true);
    await settle();
    expect(fake.paramsOf('command/list')).toEqual([{ thread: 'thread-1' }]);
    expect(commands.loaded()).toBe(true);
    panel.open.set(false);
    await settle();
    panel.open.set(true);
    await settle();
    expect(fake.paramsOf('command/list')).toHaveLength(2);
  });

  it('starts from nothing in another thread', async () => {
    const fake = new FakeSession()
      .answer('thread/events', (params) => ({
        events: (params as { thread: string }).thread === 'thread-1' ? [STARTED] : [],
      }))
      .answer('command/list', { commands: [command('c1', 'npm run dev')] });
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
      .answer('command/list', { commands: [command('c1', 'npm run dev')] })
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
      .answer('command/list', { commands: [command('c1', 'npm run dev')] });
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
    answer({ commands: [command('c1', 'npm run dev')] });
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

  it('moves its clock on each second while the commands show, and not after', async () => {
    vi.useFakeTimers({ toFake: ['Date', 'setInterval', 'clearInterval'] });
    try {
      vi.setSystemTime(Date.parse('2026-10-11T10:02:05.000Z'));
      const fake = new FakeSession()
        .answer('thread/events', { events: [STARTED] })
        .answer('command/list', { commands: [command('c1', 'npm run dev')] });
      const { commands, thread, panel } = make(fake);
      await thread.open('thread-1');
      panel.tab.set('commands');
      panel.open.set(true);
      await settle();
      const shown = commands.now();
      expect(shown).toBe(Date.parse('2026-10-11T10:02:05.000Z'));
      vi.advanceTimersByTime(COMMANDS_TICK_MS * 2);
      expect(commands.now()).toBe(shown + COMMANDS_TICK_MS * 2);
      panel.open.set(false);
      TestBed.tick();
      vi.advanceTimersByTime(COMMANDS_TICK_MS * 5);
      expect(commands.now()).toBe(shown + COMMANDS_TICK_MS * 2);
    } finally {
      vi.useRealTimers();
    }
  });
});
