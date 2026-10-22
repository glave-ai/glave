import { DOCUMENT, DestroyRef, Service, computed, inject, signal } from '@angular/core';
import {
  type ApprovalMode,
  type AttachedFile,
  type Notification,
  type RawEvent,
  type ThreadId,
  type ThreadSummary,
  type TrustAsk,
  type TrustChoice,
  type TurnStartParams,
} from '@glave/client';
import type { GlzDecision } from '@glave/glaze';

import { lastCommandChange } from '../lib/commands';
import { NO_DRAFT, applyUpdate, mergeEvents, stepsAfter, type Draft } from '../lib/conversation';
import { changedFiles } from '../lib/files';
import { chosenEffort } from '../lib/effort';
import { chosenModel } from '../lib/models';
import { placeOf } from '../lib/place';
import { folderGone } from '../lib/projects';
import { type Waiting, named, notRead, together } from '../lib/queued';
import { onApple } from '../lib/shortcuts';
import { threadTurns, turnRuns } from '../lib/turns';
import { FIRST_RETRY_MS, LAST_RETRY_MS, cutOff, pause } from '../lib/watching';
import { SessionService } from './session.service';
import { ProjectsService } from './projects.service';

/**
 * One open thread: its events, read from the log, then followed live as anyone records them,
 * from this tab, another, or the terminal, while the tab is shown; the reply being written;
 * what the person sent while a turn ran, until the model reads it; the composer's draft; what
 * its project asks the person to trust, raised by opening it; and the person's actions in it
 * (send, stop, decide, answer, trust). Each thread view provides its own.
 */
@Service({ autoProvided: false })
export class ThreadService {
  private readonly session = inject(SessionService);
  private readonly projects = inject(ProjectsService);
  private readonly id = signal<ThreadId | null>(null);
  private readonly events = signal<readonly RawEvent[]>([]);
  private readonly draft = signal<Draft>(NO_DRAFT);
  /** The steps the running turn takes now, in the server's words, as its live updates say. */
  private readonly doing = signal<readonly string[]>([]);
  /** The turn streams followed now: the turn, and any message sent to it meanwhile. */
  private readonly following = signal(0);
  private readonly waiting = signal<readonly Waiting[]>([]);
  /** What the person sent to start a turn, until the server records it. */
  private readonly asking = signal<string | null>(null);
  /** Messages the model will not read, given back once the turn has ended. */
  private givenBack: string[] = [];
  /** Whether the person stopped the turn, so what waited is sent next. */
  private stopped = false;
  /** The `seq` of the start of the turn the person stopped, which `stopped` is about. */
  private stoppedAt: number | null = null;
  /**
   * Whether a turn runs in the thread, as far as this view knows: from the server's word when it
   * opened the thread, then from each turn's start and end as they come.
   */
  private readonly running = signal(false);
  /**
   * The turns of its own this view has asked for and follows by their streams, from asking to
   * their end: its watch rests meanwhile.
   */
  private readonly streaming = signal(0);
  private sentWhileRunning = 0;
  private readonly failed = signal<string | null>(null);
  private readonly deciding = signal(false);
  private readonly asked = signal<TrustAsk | null>(null);
  private readonly trustAnswered = signal<TrustChoice | null>(null);
  private readonly controllers = new Set<AbortController>();
  /** The watch of the thread, while one runs. */
  private watching: AbortController | null = null;

  private readonly document = inject(DOCUMENT);
  private readonly mac = onApple(this.document);

  readonly thread = this.id.asReadonly();
  /** The thread's turns, each with its items, the files it changed, its time and cost. */
  readonly turns = computed(() => threadTurns(this.events()));
  /** The `seq` the thread's next event takes, as far as this view has read it. */
  readonly next = computed(() => (this.events().at(-1)?.seq ?? 0) + 1);
  /** The files the thread changed with its own `write` and `edit` calls. */
  readonly files = computed(() => changedFiles(this.events()));
  /** The `seq` of the latest start or end of a command in the background it recorded, if any. */
  readonly commandChange = computed(() => lastCommandChange(this.events()));
  readonly reply = this.draft.asReadonly();
  /** The step the running turn takes now, in the server's words, while one runs and it said so. */
  readonly step = computed(() => this.doing().at(-1) ?? null);
  /** Whether a turn runs in the thread: one this view follows, or one anyone else started. */
  readonly busy = computed(() => this.following() > 0 || this.running());
  /**
   * What the person sent to start a turn, shown at once as theirs while the server starts the
   * turn, until it records their message; `null` when nothing is on its way.
   */
  readonly starting = this.asking.asReadonly();
  /** What the composer holds: the person's next message. */
  readonly composer = signal('');
  /** What the person sent while the turn ran that the model has not read yet, oldest first. */
  readonly queued = computed(() => notRead(this.waiting(), this.events()));
  readonly error = this.failed.asReadonly();
  readonly sending = this.deciding.asReadonly();
  /** What the thread's project asks the person to trust, while it asks or once answered here. */
  readonly trust = this.asked.asReadonly();
  /** The person's answer to `trust`, once they gave it here. */
  readonly trustAnswer = this.trustAnswered.asReadonly();
  readonly hasReply = computed(() => this.draft().text !== '' || this.draft().reasoning !== '');
  /** The model the thread chose for its turns, or `null` for the server's default. */
  readonly chosen = computed(() => chosenModel(this.events()));
  /** How much the thread chose its model reasons, or `null` for the model's own way. */
  readonly chosenEffort = computed(() => chosenEffort(this.events()));
  /** Where the thread's tools act, in plain words, once it has a workspace. */
  readonly place = computed(() =>
    placeOf(this.events(), { local: this.session.local(), mac: this.mac }),
  );

  constructor() {
    // A hidden tab lets go of its watch, so tabs in the background hold no connection, and
    // comes back from where it was once shown.
    const shown = (): void => {
      const thread = this.id();
      if (this.document.visibilityState === 'hidden') {
        this.stopWatching();
      } else if (thread !== null && this.watching === null && this.streaming() === 0) {
        void this.watch(thread);
      }
    };
    this.document.addEventListener('visibilitychange', shown);
    // Leaving the thread stops following it; a turn itself goes on in the server.
    inject(DestroyRef).onDestroy(() => {
      this.document.removeEventListener('visibilitychange', shown);
      this.abortAll();
    });
  }

  /**
   * Opens `thread`: reads all its events, and what its project asks the person to trust, which
   * opening it raises.
   */
  async open(thread: ThreadId): Promise<void> {
    this.abortAll();
    this.id.set(thread);
    this.events.set([]);
    this.forgetLive();
    this.waiting.set([]);
    this.asking.set(null);
    this.givenBack = [];
    this.stopped = false;
    this.stoppedAt = null;
    this.running.set(false);
    this.failed.set(null);
    this.asked.set(null);
    this.trustAnswered.set(null);
    await this.readAll(thread);
    // Read after the events: a turn they leave open runs only when the server says so, not
    // when glave stopped in the middle of it.
    const summary = await this.readAsk(thread);
    if (this.id() !== thread) {
      return;
    }
    this.running.set(summary !== null && runs(summary, this.events()));
    if (this.document.visibilityState !== 'hidden') {
      void this.watch(thread, true);
    }
  }

  /**
   * Answers what the thread's project asks: trusting turns it on from the next turn; not now
   * keeps it off. Either way the card rests, and the side nav hears of it from the server.
   */
  async answerTrust(answer: TrustChoice): Promise<void> {
    const thread = this.id();
    const ask = this.asked();
    if (thread === null || ask === null || this.trustAnswered() !== null) {
      return;
    }
    this.deciding.set(true);
    try {
      await this.session.call('trust/answer', { thread, ask: ask.ask, answer });
      this.trustAnswered.set(answer);
      this.failed.set(null);
    } catch (error: unknown) {
      this.failed.set(error instanceof Error ? error.message : String(error));
      // The project may have changed since it asked: show what it asks now.
      await this.readAsk(thread);
    } finally {
      this.deciding.set(false);
    }
  }

  /**
   * Sends `prompt`, with the `files` kept for it, and follows the turn as it runs. With
   * `model`, the turn and the thread's turns after it go to that model; with `approval`, they
   * run in that approval mode; with `effort`, their model reasons that much. While a turn runs,
   * the message goes to it instead, for the model to read at its next step, and waits in the
   * thread until it has; the model, mode and level go with the next turn.
   */
  async send(
    prompt: string,
    model: string | null = null,
    approval: ApprovalMode | null = null,
    files: readonly AttachedFile[] = [],
    effort: string | null = null,
  ): Promise<void> {
    const thread = this.id();
    if (thread === null) {
      return;
    }
    if (this.busy()) {
      this.sentWhileRunning += 1;
      const key = `queued-${String(this.sentWhileRunning)}`;
      this.waiting.update((waiting) => [...waiting, { key, text: prompt, id: null }]);
      await this.follow(thread, { thread, prompt, ...withFiles(files) }, key);
      return;
    }
    const params: TurnStartParams = {
      thread,
      prompt,
      ...withFiles(files),
      ...(model === null ? {} : { model }),
      ...(approval === null ? {} : { approval }),
      ...(effort === null ? {} : { effort }),
    };
    await this.follow(thread, params, null);
  }

  /** Stops the running turn; what the person sent meanwhile and the model never read is sent next. */
  async stop(): Promise<void> {
    const thread = this.id();
    if (thread !== null) {
      this.stopped = this.busy();
      this.stoppedAt = latestStart(this.events());
      await this.act(() => this.session.call('turn/cancel', { thread }));
    }
  }

  /** Gives the thread a new title, which the side nav hears of from the server. */
  async rename(title: string): Promise<void> {
    const thread = this.id();
    if (thread === null) {
      return;
    }
    try {
      await this.session.call('thread/rename', { thread, title });
      this.failed.set(null);
    } catch (error: unknown) {
      this.failed.set(error instanceof Error ? error.message : String(error));
    }
  }

  /** Answers an approval; the paused turn goes on. */
  async decide(approval: string, decision: GlzDecision): Promise<void> {
    const thread = this.id();
    if (thread !== null) {
      await this.act(() => this.session.call('approval/decide', { thread, approval, decision }));
    }
  }

  /** Answers a question call; the paused turn goes on. */
  async answer(question: string, answers: readonly string[]): Promise<void> {
    const thread = this.id();
    if (thread !== null) {
      await this.act(() => this.session.call('question/answer', { thread, question, answers }));
    }
  }

  /** Reads what was recorded since the last event this view has. */
  async catchUp(): Promise<void> {
    const thread = this.id();
    if (thread !== null) {
      await this.readAll(thread, this.events().at(-1)?.seq);
    }
  }

  /**
   * Sends `params` and follows what comes back: a turn of its own, streamed, or for `queued`,
   * a message the server queued to the turn already running, answered at once with the id it
   * will be recorded under.
   */
  private async follow(
    thread: ThreadId,
    params: TurnStartParams,
    queued: string | null,
  ): Promise<void> {
    const controller = new AbortController();
    this.controllers.add(controller);
    this.following.update((streams) => streams + 1);
    this.failed.set(null);
    // A turn of its own is followed by its own stream: the watch rests meanwhile, unless the
    // message is likely to be queued to a turn already running.
    let own = queued === null;
    if (own) {
      this.asking.set(params.prompt);
      this.streaming.update((streams) => streams + 1);
      if (!this.running()) {
        this.stopWatching();
      }
    }
    try {
      const turn = this.session.stream('turn/start', params, controller.signal);
      let next = await turn.next();
      if (!next.done) {
        if (!own) {
          own = true;
          this.streaming.update((streams) => streams + 1);
        }
        this.stopWatching();
        // The turn had ended meanwhile, so the message started one of its own.
        this.waiting.update((waiting) => waiting.filter((message) => message.key !== queued));
      }
      while (!next.done) {
        this.take(thread, next.value, 'turn');
        next = await turn.next();
      }
      const result = next.value;
      if ('queued' in result) {
        this.keepWaiting(queued, params.prompt, result.queued.id);
      }
    } catch (error: unknown) {
      if (folderGone(error)) {
        // The project's folder went since the lists were read: they say so in its place.
        void this.projects.refresh();
      } else if (!controller.signal.aborted) {
        const said = error instanceof Error ? error.message : String(error);
        this.failed.set(said);
        // Refused, as when too many wait: back in the box at once, saying why.
        this.giveBack(queued, said);
        this.giveBackStart(queued);
      }
    } finally {
      // Not when it was let go of, on leaving the thread.
      if (this.controllers.delete(controller)) {
        if (queued === null) {
          this.asking.set(null);
        }
        this.following.update((streams) => streams - 1);
        if (own) {
          this.streaming.update((streams) => streams - 1);
        }
        // A message queued to a turn another tab follows waits for that turn, as its draft does.
        if (this.following() === 0 && !this.running()) {
          this.settle(thread);
        }
        if (this.streaming() === 0 && this.watching === null) {
          this.watchIfShown(thread);
        }
      }
    }
  }

  /**
   * Keeps the message the server queued under `id` waiting until the model reads it: the one
   * sent while a turn ran (`key`), or one sent to start a turn that another had started first.
   */
  private keepWaiting(key: string | null, text: string, id: string): void {
    if (key !== null) {
      this.waiting.update((waiting) => named(waiting, key, id));
      return;
    }
    this.sentWhileRunning += 1;
    const queued = `queued-${String(this.sentWhileRunning)}`;
    this.waiting.update((waiting) => [...waiting, { key: queued, text, id }]);
    // Queued: a turn runs that this view has not seen start yet.
    this.running.set(true);
  }

  /**
   * Takes one notification about `thread`, from a turn this view follows or from its watch.
   * The reply being written is drawn from one of them: the turn's stream while it follows a
   * turn of its own, else the watch; a message recorded there takes its place.
   */
  private take(thread: ThreadId, note: Notification, from: 'turn' | 'watch'): void {
    if (note.params.thread !== thread) {
      return;
    }
    const drawn = from === 'turn' || this.streaming() === 0;
    if (note.method === 'thread/event') {
      const event = note.params.event;
      // A thread's events are numbered without gaps: one missed between two streams is read.
      const newest = this.events().at(-1)?.seq;
      if (newest !== undefined && event.seq > newest + 1) {
        void this.readAll(thread, newest, true);
      }
      const added = this.add(event);
      if (drawn && event.type === 'message.added') {
        this.forgetLive();
      }
      if (added && event.type === 'turn.ended' && this.streaming() === 0) {
        this.endedElsewhere();
      }
    } else if (note.method === 'turn/live' && drawn) {
      const update = note.params.update;
      if (update.kind === 'unread') {
        const given = this.waiting().find((message) => message.id === update.id);
        this.giveBack(given?.key ?? null, update.reason ?? null);
        if (!this.busy()) {
          this.returnGivenBack(thread);
        }
      } else {
        this.draft.update((draft) => applyUpdate(draft, update));
        this.doing.update((steps) => stepsAfter(steps, update));
      }
    }
  }

  /** A turn this view did not follow ended: its draft goes. */
  private endedElsewhere(): void {
    this.forgetLive();
  }

  /** What a turn showed live goes, the reply being written and the steps it took: never kept. */
  private forgetLive(): void {
    this.draft.set(NO_DRAFT);
    this.doing.set([]);
  }

  /**
   * What the model never read, given back after a turn this view did not follow had ended:
   * sent next when the person stopped it here, else back in the box.
   */
  private returnGivenBack(thread: ThreadId): void {
    const unread = this.givenBack;
    this.givenBack = [];
    if (unread.length === 0) {
      return;
    }
    if (this.stopped && this.stoppedAt === latestStart(this.events())) {
      this.stopped = false;
      void this.follow(thread, { thread, prompt: together(unread) }, null);
    } else {
      this.composer.update((draft) => together([...unread, draft].filter(Boolean)));
    }
  }

  /**
   * Follows `thread` live from the last event this view has: what any client records in it, and
   * the live updates of a turn another tab or the terminal follows, its approvals and questions
   * among them. Cut off, it comes back from where it was, waiting longer each time; it ends once
   * the thread is deleted, the tab signs out, or the view lets go of it.
   */
  private async watch(thread: ThreadId, read = false): Promise<void> {
    this.stopWatching();
    const controller = new AbortController();
    this.watching = controller;
    const stopped = (): boolean => controller.signal.aborted;
    let [wait, fresh] = [FIRST_RETRY_MS, read];
    while (!stopped()) {
      // Live updates start afresh with each watch: a reply's text missed meanwhile is gone.
      this.forgetLive();
      // Just read, the thread needs no second look.
      if (!fresh) {
        await this.resync(thread);
      }
      fresh = false;
      const began = Date.now();
      const after = this.events().at(-1)?.seq;
      try {
        const params = { thread, ...(after === undefined ? {} : { after }) };
        const watch = this.session.stream('thread/watch', params, controller.signal);
        let next = await watch.next();
        while (!next.done) {
          this.take(thread, next.value, 'watch');
          wait = FIRST_RETRY_MS;
          next = await watch.next();
        }
        if (next.value.ended === 'deleted') {
          break;
        }
      } catch (error: unknown) {
        if (stopped() || !cutOff(error)) {
          break;
        }
      }
      // A watch that lasted was not failing: the next waits only a little.
      if (Date.now() - began > 2 * wait) {
        wait = FIRST_RETRY_MS;
      }
      await pause(wait, controller.signal);
      wait = Math.min(wait * 2, LAST_RETRY_MS);
    }
    if (this.watching === controller) {
      this.watching = null;
    }
  }

  private stopWatching(): void {
    this.watching?.abort();
    this.watching = null;
  }

  private watchIfShown(thread: ThreadId): void {
    if (this.document.visibilityState !== 'hidden') {
      void this.watch(thread);
    }
  }

  /**
   * Before a watch comes back, while the view believes a turn runs or has sent messages the
   * model has not read: it may have missed that turn's end, or the turn giving them back, since
   * live updates are never replayed and a turn left open by a crash ends only later. The
   * server's word on whether a turn runs comes first, then the events recorded since; with no
   * turn running, what waited goes back in the box.
   */
  private async resync(thread: ThreadId): Promise<void> {
    const stranded = (): readonly string[] =>
      this.queued()
        .filter((message) => message.id !== null)
        .map((message) => message.text);
    if (this.following() > 0 || (!this.running() && stranded().length === 0)) {
      return;
    }
    const summary = await this.readAsk(thread, true);
    if (summary === null || this.following() > 0 || this.id() !== thread) {
      return;
    }
    // The server's word first; a turn's start or end read after it says more.
    this.running.set(runs(summary, this.events()));
    await this.readAll(thread, this.events().at(-1)?.seq, true);
    if (this.following() > 0 || this.id() !== thread) {
      return;
    }
    const unread = this.running() ? [] : stranded();
    if (unread.length > 0) {
      this.waiting.update((waiting) => waiting.filter((message) => message.id === null));
      this.composer.update((draft) => together([...unread, draft].filter(Boolean)));
    }
  }

  /**
   * Takes the message `key` back from the thread: `reason` says why the model will not read it
   * (a hook stopped it, or the server refused it), and it goes back in the box; without one the
   * turn ended first, and it is kept until the turn has.
   */
  private giveBack(key: string | null, reason: string | null): void {
    const message = this.waiting().find((each) => each.key === key);
    if (message === undefined) {
      return;
    }
    this.waiting.update((waiting) => waiting.filter((each) => each.key !== key));
    if (reason === null) {
      this.givenBack.push(message.text);
    } else {
      this.failed.set(reason);
      this.composer.update((draft) => together([message.text, draft].filter(Boolean)));
    }
  }

  /** A turn's start the server refused before recording it: what the person sent goes back in the box. */
  private giveBackStart(queued: string | null): void {
    const prompt = this.asking();
    if (queued !== null || prompt === null) {
      return;
    }
    this.asking.set(null);
    this.composer.update((draft) => together([prompt, draft].filter(Boolean)));
  }

  /**
   * Once nothing is followed any more:what the model never read is sent next when the person
   * stopped the turn, as Claude Code and Codex do; otherwise it goes back in the box.
   */
  private settle(thread: ThreadId): void {
    this.forgetLive();
    const unread = [...this.givenBack, ...this.queued().map((message) => message.text)];
    this.givenBack = [];
    this.waiting.set([]);
    const stopped = this.stopped;
    this.stopped = false;
    if (unread.length === 0) {
      return;
    }
    if (stopped) {
      void this.follow(thread, { thread, prompt: together(unread) }, null);
    } else {
      this.composer.update((draft) => together([...unread, draft].filter(Boolean)));
    }
  }

  private abortAll(): void {
    this.stopWatching();
    for (const controller of this.controllers) {
      controller.abort();
    }
    this.controllers.clear();
    this.following.set(0);
    this.streaming.set(0);
  }

  private async act(call: () => Promise<unknown>): Promise<void> {
    this.deciding.set(true);
    try {
      await call();
      // A turn this view is not following (opened while it waited) goes on in the server.
      if (!this.busy()) {
        await this.catchUp();
      }
    } catch (error: unknown) {
      this.failed.set(error instanceof Error ? error.message : String(error));
    } finally {
      this.deciding.set(false);
    }
  }

  /** Reads the thread's summary, which raises its project's ask, and shows the ask: the summary. */
  private async readAsk(thread: ThreadId, quiet = false): Promise<ThreadSummary | null> {
    try {
      const { thread: summary } = await this.session.call('thread/get', { thread });
      if (this.id() !== thread) {
        return null;
      }
      this.asked.set(summary.trust ?? null);
      return summary;
    } catch (error: unknown) {
      if (!quiet) {
        this.failed.set(error instanceof Error ? error.message : String(error));
      }
      return null;
    }
  }

  /**
   * Adds `event`, unless this view has it already, as a turn's stream and the watch may both
   * send it: whether it is new. A turn runs from its start to its end.
   */
  private add(event: RawEvent): boolean {
    const newest = this.events().at(-1)?.seq ?? 0;
    if (event.seq <= newest && this.events().some((known) => known.seq === event.seq)) {
      return false;
    }
    this.events.update((events) => mergeEvents(events, [event]));
    if (event.type === 'turn.started') {
      this.running.set(true);
      // A turn after the one the person stopped is not the one they stopped.
      if (this.stoppedAt !== null && event.seq > this.stoppedAt) {
        this.stopped = false;
        this.stoppedAt = null;
      }
    } else if (event.type === 'turn.ended') {
      this.running.set(false);
    } else if (event.type === 'message.added' && event.data['role'] === 'user') {
      this.asking.set(null);
    }
    return true;
  }

  /** Reads `thread`'s events after `from`, saying why it could not unless `quiet`. */
  private async readAll(thread: ThreadId, from?: number, quiet = false): Promise<void> {
    try {
      let after = from;
      for (;;) {
        const page = await this.session.call('thread/events', {
          thread,
          ...(after === undefined ? {} : { after }),
        });
        if (this.id() !== thread) {
          return;
        }
        this.events.update((events) => mergeEvents(events, page.events));
        const turns = page.events.some(
          (event) => event.type === 'turn.started' || event.type === 'turn.ended',
        );
        if (turns) {
          this.running.set(turnRuns(this.events()));
        }
        if (page.next === undefined || page.next === null) {
          return;
        }
        after = page.next;
      }
    } catch (error: unknown) {
      if (!quiet) {
        this.failed.set(error instanceof Error ? error.message : String(error));
      }
    }
  }
}

/**
 * Whether a turn runs in the thread, as the server's summary of it says, with its events read
 * before: one does, or waits for the person (not a project's ask, which waits with no turn).
 */
function runs(summary: ThreadSummary, events: readonly RawEvent[]): boolean {
  return summary.status === 'running' || (summary.status === 'waiting' && turnRuns(events));
}

/** The `seq` of the latest turn's start, if a turn ever started. */
function latestStart(events: readonly RawEvent[]): number | null {
  for (let at = events.length - 1; at >= 0; at -= 1) {
    const event = events[at];
    if (event?.type === 'turn.started') {
      return event.seq;
    }
  }
  return null;
}

/** `files` as `turn/start` takes them: left out when there are none. */
function withFiles(files: readonly AttachedFile[]): Pick<TurnStartParams, 'files'> {
  return files.length === 0 ? {} : { files };
}
