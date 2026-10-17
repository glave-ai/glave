import { DOCUMENT, DestroyRef, Service, computed, inject, signal } from '@angular/core';
import type {
  ApprovalMode,
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
import { onApple } from '../lib/shortcuts';
import { threadTurns } from '../lib/turns';
import { SessionService } from './session.service';
import { SpacesService } from './spaces.service';

/**
 * One open thread: its events, read from the log and followed live while a turn runs; the
 * reply being written; what its project asks the person to trust, raised by opening it; and
 * the person's actions in it (send, stop, decide, answer, trust). Each thread view provides
 * its own.
 */
@Service({ autoProvided: false })
export class ThreadService {
  private readonly session = inject(SessionService);
  private readonly spaces = inject(SpacesService);
  private readonly id = signal<ThreadId | null>(null);
  private readonly events = signal<readonly RawEvent[]>([]);
  private readonly draft = signal<Draft>(NO_DRAFT);
  private readonly running = signal(false);
  private readonly failed = signal<string | null>(null);
  private readonly deciding = signal(false);
  private readonly asked = signal<TrustAsk | null>(null);
  private readonly trustAnswered = signal<TrustChoice | null>(null);
  private controller: AbortController | null = null;

  private readonly mac = onApple(inject(DOCUMENT));

  readonly thread = this.id.asReadonly();
  /** The thread's turns, each with its items, the files it changed, its time and cost. */
  readonly turns = computed(() => threadTurns(this.events()));
  /** The files the thread changed with its own `write` and `edit` calls. */
  readonly files = computed(() => changedFiles(this.events()));
  /** The `seq` of the latest start or end of a command in the background it recorded, if any. */
  readonly commandChange = computed(() => lastCommandChange(this.events()));
  readonly reply = this.draft.asReadonly();
  readonly busy = this.running.asReadonly();
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
      this.controller?.abort();
    });
  }

  /**
   * Opens `thread`: reads all its events, and what its project asks the person to trust, which
   * opening it raises.
   */
  async open(thread: ThreadId): Promise<void> {
    this.controller?.abort();
    this.id.set(thread);
    this.events.set([]);
    this.draft.set(NO_DRAFT);
    this.running.set(false);
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
   * turns after it go to that model; with `approval`, they run in that approval mode.
   */
  async send(
    prompt: string,
    model: string | null = null,
    approval: ApprovalMode | null = null,
  ): Promise<void> {
    const thread = this.id();
    if (thread === null || this.running()) {
      return;
    }
    const controller = new AbortController();
    this.controller = controller;
    this.running.set(true);
    this.failed.set(null);
    try {
      const params: TurnStartParams = {
        thread,
        prompt,
        ...(model === null ? {} : { model }),
        ...(approval === null ? {} : { approval }),
      };
      const turn = this.session.stream('turn/start', params, controller.signal);
      for await (const note of turn) {
        if (note.params.thread !== thread) {
          continue;
        }
        if (note.method === 'thread/event') {
          this.add(note.params.event);
        } else if (note.method === 'turn/live') {
          this.draft.update((draft) => applyUpdate(draft, note.params.update));
        }
      }
    } catch (error: unknown) {
      if (!controller.signal.aborted) {
        this.failed.set(error instanceof Error ? error.message : String(error));
      }
    } finally {
      this.running.set(false);
      this.draft.set(NO_DRAFT);
      void this.spaces.refresh();
    }
  }

  /** Stops the running turn. */
  async stop(): Promise<void> {
    const thread = this.id();
    if (thread !== null) {
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

  private async act(call: () => Promise<unknown>): Promise<void> {
    this.deciding.set(true);
    try {
      await call();
      // A turn this view is not following (opened while it waited) goes on in the server.
      if (!this.running()) {
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
