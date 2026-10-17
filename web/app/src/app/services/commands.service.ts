import { Service, computed, effect, inject, linkedSignal, signal, untracked } from '@angular/core';
import type { CommandId, RunningCommand, ThreadId } from '@glave/client';

import { alreadyEnded, commandProblem } from '../lib/commands';
import { SessionService } from './session.service';
import { ThreadService } from './thread.service';
import { ToolsPanelService } from './tools-panel.service';

/** How often the time each command has run moves on while they show, in milliseconds. */
export const COMMANDS_TICK_MS = 1_000;

/** What is known of the open thread's commands in the background. */
interface State {
  readonly commands: readonly RunningCommand[];
  /** Whether they have been read once for this thread. */
  readonly loaded: boolean;
  /** Why they could not be read, in words for the person. */
  readonly error: string | null;
  /** The commands being stopped. */
  readonly stopping: ReadonlySet<CommandId>;
  /** Why a command could not be stopped, by command. */
  readonly refusals: ReadonlyMap<CommandId, string>;
}

const NOTHING_YET: State = {
  commands: [],
  loaded: false,
  error: null,
  stopping: new Set(),
  refusals: new Map(),
};

function without<K, V>(map: ReadonlyMap<K, V>, key: K): ReadonlyMap<K, V> {
  return new Map([...map].filter(([each]) => each !== key));
}

/**
 * The open thread's commands running in the background (`command/list`), and stopping one
 * (`command/stop`): the row shows it stopping, then goes; a command that had already ended goes
 * too, and any other refusal stays on its row in the server's words. Read again whenever the
 * thread records one starting or ending, and when the tools panel opens on them; no timer
 * reads them, since the thread's events say when they change. While they show, a clock moves
 * on each second for how long each has run. Each thread view provides its own.
 */
@Service({ autoProvided: false })
export class CommandsService {
  private readonly session = inject(SessionService);
  private readonly thread = inject(ThreadService);
  private readonly panel = inject(ToolsPanelService);
  /** Starts again from nothing in each thread. */
  private readonly state = linkedSignal<ThreadId | null, State>({
    source: this.thread.thread,
    computation: () => NOTHING_YET,
  });
  private readonly clock = signal(Date.now());
  /** Counts the reads and stops begun, so an answer that a later one overtook is dropped. */
  private latest = 0;

  readonly commands = computed(() => this.state().commands);
  readonly loaded = computed(() => this.state().loaded);
  readonly error = computed(() => this.state().error);
  readonly stopping = computed(() => this.state().stopping);
  readonly refusals = computed(() => this.state().refusals);
  readonly count = computed(() => this.commands().length);
  /** Now, in milliseconds since the epoch, moving on each second while the commands show. */
  readonly now = this.clock.asReadonly();
  /** Whether a command shows how long it has run, which only a recorded start says. */
  private readonly timed = computed(() =>
    this.commands().some((command) => typeof command.started_at === 'string'),
  );

  constructor() {
    // The thread's history says when a command started or ended, live while a turn streams in.
    effect(() => {
      const thread = this.thread.thread();
      const change = this.thread.commandChange();
      if (thread !== null && change !== null) {
        untracked(() => void this.refresh());
      }
    });
    effect(() => {
      const thread = this.thread.thread();
      if (thread !== null && this.panel.commandsShown()) {
        untracked(() => void this.refresh());
      }
    });
    effect((onCleanup) => {
      if (!this.panel.commandsShown() || !this.timed()) {
        return;
      }
      this.clock.set(Date.now());
      const timer = setInterval(() => {
        this.clock.set(Date.now());
      }, COMMANDS_TICK_MS);
      onCleanup(() => {
        clearInterval(timer);
      });
    });
  }

  /** Reads the thread's commands running now. */
  async refresh(): Promise<void> {
    const thread = this.thread.thread();
    if (thread === null) {
      return;
    }
    this.latest += 1;
    const ticket = this.latest;
    const current = (): boolean => ticket === this.latest && this.thread.thread() === thread;
    try {
      const { commands } = await this.session.call('command/list', { thread });
      if (current()) {
        this.state.update((state) => ({ ...state, commands, loaded: true, error: null }));
      }
    } catch (error: unknown) {
      if (current()) {
        this.state.update((state) => ({ ...state, loaded: true, error: commandProblem(error) }));
      }
    }
  }

  /** Stops `command`, which shows as stopping until the server answers. */
  async stop(command: CommandId): Promise<void> {
    const thread = this.thread.thread();
    if (thread === null || this.stopping().has(command)) {
      return;
    }
    this.state.update((state) => ({
      ...state,
      stopping: new Set([...state.stopping, command]),
      refusals: without(state.refusals, command),
    }));
    let refusal: string | null = null;
    try {
      await this.session.call('command/stop', { thread, command });
    } catch (error: unknown) {
      refusal = alreadyEnded(error) ? null : commandProblem(error);
    }
    if (this.thread.thread() !== thread) {
      return;
    }
    const stopping = new Set([...this.stopping()].filter((each) => each !== command));
    if (refusal !== null) {
      const refusals = new Map([...this.refusals(), [command, refusal]]);
      this.state.update((state) => ({ ...state, stopping, refusals }));
      return;
    }
    // It no longer runs: that overtakes any read begun before the server said so.
    this.latest += 1;
    this.state.update((state) => ({
      ...state,
      commands: state.commands.filter((each) => each.id !== command),
      stopping,
    }));
  }
}
