import { DOCUMENT, DestroyRef, Service, computed, inject, signal } from '@angular/core';
import type {
  ApprovalMode,
  Notification,
  RawEvent,
  ThreadId,
  TrustAsk,
  TrustChoice,
  TurnStartParams,
} from '@glave/client';
import type { GlzDecision } from '@glave/glaze';

import { lastCommandChange } from '../lib/commands';
import { NO_DRAFT, applyUpdate, mergeEvents, type Draft } from '../lib/conversation';
import { changedFiles } from '../lib/files';
import { chosenModel } from '../lib/models';
import { placeOf } from '../lib/place';
import { folderGone } from '../lib/projects';
import { type Waiting, named, notRead, together } from '../lib/queued';
import { onApple } from '../lib/shortcuts';
import { threadTurns } from '../lib/turns';
import { SessionService } from './session.service';
import { SpacesService } from './spaces.service';

/**
 * One open thread: its events, read from the log and followed live while a turn runs; the
 * reply being written; what the person sent while it ran, until the model reads it; the
 * composer's draft; what its project asks the person to trust, raised by opening it; and the
 * person's actions in it (send, stop, decide, answer, trust). Each thread view provides its
 * own.
 */
@Service({ autoProvided: false })
export class ThreadService {
  private readonly session = inject(SessionService);
  private readonly spaces = inject(SpacesService);
  private readonly id = signal<ThreadId | null>(null);
  private readonly events = signal<readonly RawEvent[]>([]);
  private readonly draft = signal<Draft>(NO_DRAFT);
  /** The turn streams followed now: the turn, and any message sent to it meanwhile. */
  private readonly following = signal(0);
  private readonly waiting = signal<readonly Waiting[]>([]);
  /** Messages the model will not read, given back once the turn has ended. */
  private givenBack: string[] = [];
  /** Whether the person stopped the turn, so what waited is sent next. */
  private stopped = false;
  private sentWhileRunning = 0;
  private readonly failed = signal<string | null>(null);
  private readonly deciding = signal(false);
  private readonly asked = signal<TrustAsk | null>(null);
  private readonly trustAnswered = signal<TrustChoice | null>(null);
  private readonly controllers = new Set<AbortController>();

  private readonly mac = onApple(inject(DOCUMENT));

  readonly thread = this.id.asReadonly();
  /** The thread's turns, each with its items, the files it changed, its time and cost. */
  readonly turns = computed(() => threadTurns(this.events()));
  /** The files the thread changed with its own `write` and `edit` calls. */
  readonly files = computed(() => changedFiles(this.events()));
  /** The `seq` of the latest start or end of a command in the background it recorded, if any. */
  readonly commandChange = computed(() => lastCommandChange(this.events()));
  readonly reply = this.draft.asReadonly();
  readonly busy = computed(() => this.following() > 0);
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
  /** Where the thread's tools act, in plain words, once it has a workspace. */
  readonly place = computed(() =>
    placeOf(this.events(), { local: this.session.local(), mac: this.mac }),
  );

  constructor() {
    // Leaving the thread stops following its turn; the turn itself goes on in the server.
    inject(DestroyRef).onDestroy(() => {
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
    this.draft.set(NO_DRAFT);
    this.waiting.set([]);
    this.givenBack = [];
    this.stopped = false;
    this.failed.set(null);
    this.asked.set(null);
    this.trustAnswered.set(null);
    await Promise.all([this.readAsk(thread), this.readAll(thread)]);
  }

  /**
   * Answers what the thread's project asks: trusting turns it on from the next turn; not now
   * keeps it off. Either way the card rests, and the side nav's lists are read again.
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
      void this.spaces.refresh();
    }
  }

  /**
   * Sends `prompt` and follows the turn as it runs. With `model`, the turn and the thread's
   * turns after it go to that model; with `approval`, they run in that approval mode. While a
   * turn runs, the message goes to it instead, for the model to read at its next step, and
   * waits in the thread until it has; the model and mode go with the next turn.
   */
  async send(
    prompt: string,
    model: string | null = null,
    approval: ApprovalMode | null = null,
  ): Promise<void> {
    const thread = this.id();
    if (thread === null) {
      return;
    }
    if (this.busy()) {
      this.sentWhileRunning += 1;
      const key = `queued-${String(this.sentWhileRunning)}`;
      this.waiting.update((waiting) => [...waiting, { key, text: prompt, id: null }]);
      await this.follow(thread, { thread, prompt }, key);
      return;
    }
    const params: TurnStartParams = {
      thread,
      prompt,
      ...(model === null ? {} : { model }),
      ...(approval === null ? {} : { approval }),
    };
    await this.follow(thread, params, null);
  }

  /** Stops the running turn; what the person sent meanwhile and the model never read is sent next. */
  async stop(): Promise<void> {
    const thread = this.id();
    if (thread !== null) {
      this.stopped = this.busy();
      await this.act(() => this.session.call('turn/cancel', { thread }));
    }
  }

  /** Gives the thread a new title, then reads the side nav's lists again. */
  async rename(title: string): Promise<void> {
    const thread = this.id();
    if (thread === null) {
      return;
    }
    try {
      await this.session.call('thread/rename', { thread, title });
      this.failed.set(null);
      await this.spaces.refresh();
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
    try {
      const turn = this.session.stream('turn/start', params, controller.signal);
      let next = await turn.next();
      if (!next.done && queued !== null) {
        // The turn had ended meanwhile, so the message started one of its own.
        this.waiting.update((waiting) => waiting.filter((message) => message.key !== queued));
      }
      while (!next.done) {
        this.take(thread, next.value);
        next = await turn.next();
      }
      const result = next.value;
      if (queued !== null && 'queued' in result) {
        this.waiting.update((waiting) => named(waiting, queued, result.queued.id));
      }
    } catch (error: unknown) {
      if (folderGone(error)) {
        // The project's folder went since the lists were read: they say so in its place.
        void this.spaces.refresh();
      } else if (!controller.signal.aborted) {
        const said = error instanceof Error ? error.message : String(error);
        this.failed.set(said);
        // Refused, as when too many wait: back in the box at once, saying why.
        this.giveBack(queued, said);
      }
    } finally {
      // Not when it was let go of, on leaving the thread.
      if (this.controllers.delete(controller)) {
        this.following.update((streams) => streams - 1);
        if (this.following() === 0) {
          this.settle(thread);
        }
      }
    }
  }

  /** Takes one notification of the turn followed in `thread`. */
  private take(thread: ThreadId, note: Notification): void {
    if (note.params.thread !== thread) {
      return;
    }
    if (note.method === 'thread/event') {
      this.add(note.params.event);
    } else if (note.method === 'turn/live') {
      const update = note.params.update;
      if (update.kind === 'unread') {
        const given = this.waiting().find((message) => message.id === update.id);
        this.giveBack(given?.key ?? null, update.reason ?? null);
      } else {
        this.draft.update((draft) => applyUpdate(draft, update));
      }
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

  /**
   * Once nothing is followed any more: what the model never read is sent next when the person
   * stopped the turn, as Claude Code and Codex do; otherwise it goes back in the box.
   */
  private settle(thread: ThreadId): void {
    this.draft.set(NO_DRAFT);
    void this.spaces.refresh();
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
    for (const controller of this.controllers) {
      controller.abort();
    }
    this.controllers.clear();
    this.following.set(0);
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

  /** Reads the thread's summary, which raises its project's ask, and shows the ask. */
  private async readAsk(thread: ThreadId): Promise<void> {
    try {
      const { thread: summary } = await this.session.call('thread/get', { thread });
      if (this.id() !== thread) {
        return;
      }
      const ask = summary.trust ?? null;
      this.asked.set(ask);
      if (ask !== null) {
        void this.spaces.refresh();
      }
    } catch (error: unknown) {
      this.failed.set(error instanceof Error ? error.message : String(error));
    }
  }

  private add(event: RawEvent): void {
    this.events.update((events) => mergeEvents(events, [event]));
    if (event.type === 'message.added') {
      this.draft.set(NO_DRAFT);
    }
  }

  private async readAll(thread: ThreadId, from?: number): Promise<void> {
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
        if (page.next === undefined || page.next === null) {
          return;
        }
        after = page.next;
      }
    } catch (error: unknown) {
      this.failed.set(error instanceof Error ? error.message : String(error));
    }
  }
}
