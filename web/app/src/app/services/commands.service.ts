import { Service, computed, effect, inject, linkedSignal, untracked } from '@angular/core';
import type { CommandId, RunningCommand, ThreadId } from '@glave/client';

import { alreadyEnded, commandProblem } from '../lib/commands';
import { clock } from './clock';
import { SessionService } from './session.service';
import { ThreadService } from './thread.service';
import { PanelService } from './panel.service';

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
 * too, and any other refusal stays on its row in the server's words. Read once the thread has
 * recorded one starting, again whenever it records one starting or ending, and each time its
 * panel is shown again; no timer reads them, since the thread's events say when they change.
 * While the panel shows, a clock moves on each second for how long each has run. Each thread
 * view provides its own.
 */
@Service({ autoProvided: false })
export class CommandsService {
  private readonly session = inject(SessionService);
  private readonly thread = inject(ThreadService);
  private readonly panel = inject(PanelService);
  /** Starts again from nothing in each thread. */
  private readonly state = linkedSignal<ThreadId | null, State>({
    source: this.thread.thread,
    computation: () => NOTHING_YET,
  });
  /** Counts the reads and stops begun, so an answer that a later one overtook is dropped. */
  private latest = 0;

  readonly commands = computed(() => this.state().commands);
  readonly loaded = computed(() => this.state().loaded);
  /** Whether what the thread's history calls for has been read, or there is nothing to read. */
  readonly known = computed(() => this.thread.commandChange() === null || this.loaded());
  readonly error = computed(() => this.state().error);
  readonly stopping = computed(() => this.state().stopping);
  readonly refusals = computed(() => this.state().refusals);
  readonly count = computed(() => this.commands().length);
  /** Whether a command shows how long it has run, which only a recorded start says. */
  private readonly timed = computed(() =>
    this.commands().some((command) => typeof command.started_at === 'string'),
  );
  /** Now, in milliseconds since the epoch, moving on each second while the commands show. */
  readonly now = clock(() => this.panel.showing() && this.timed());

  constructor() {
    // The thread's history says when a command started or ended, live while a turn streams in.
    effect(() => {
      const thread = this.thread.thread();
      const change = this.thread.commandChange();
      if (thread !== null && change !== null) {
        untracked(() => void this.refresh());
      }
    });
    // Shown again, the panel reads them again: one may have ended with nothing recorded yet.
    let shown = untracked(this.panel.showing);
    effect(() => {
      const open = this.panel.showing();
      if (open && !shown && untracked(this.thread.commandChange) !== null) {
        untracked(() => void this.refresh());
      }
      shown = open;
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
