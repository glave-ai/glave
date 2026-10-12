import { DestroyRef, Service, computed, inject, signal } from '@angular/core';
import type { ApprovalMode, RawEvent, ThreadId, TurnStartParams } from '@glave/client';
import type { GlzDecision } from '@glave/glaze';

import {
  NO_DRAFT,
  applyUpdate,
  latestModel,
  mergeEvents,
  threadItems,
  type Draft,
} from '../lib/conversation';
import { chosenModel } from '../lib/models';
import { SessionService } from './session.service';
import { SpacesService } from './spaces.service';

/**
 * One open thread: its events, read from the log and followed live while a turn runs; the
 * reply being written; and the person's actions in it (send, stop, decide, answer). Each
 * thread view provides its own.
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
  private controller: AbortController | null = null;

  readonly thread = this.id.asReadonly();
  /** The thread's events, in `seq` order. */
  readonly log = this.events.asReadonly();
  readonly items = computed(() => threadItems(this.events()));
  readonly reply = this.draft.asReadonly();
  readonly busy = this.running.asReadonly();
  readonly error = this.failed.asReadonly();
  readonly sending = this.deciding.asReadonly();
  /** The model the latest turn runs on, for the reply streaming in. */
  readonly model = computed(() => latestModel(this.events()));
  readonly hasReply = computed(() => this.draft().text !== '' || this.draft().reasoning !== '');
  /** The model the thread chose for its turns, or `null` for the server's default. */
  readonly chosen = computed(() => chosenModel(this.events()));

  constructor() {
    // Leaving the thread stops following its turn; the turn itself goes on in the server.
    inject(DestroyRef).onDestroy(() => {
      this.controller?.abort();
    });
  }

  /** Opens `thread`: reads all its events. */
  async open(thread: ThreadId): Promise<void> {
    this.controller?.abort();
    this.id.set(thread);
    this.events.set([]);
    this.draft.set(NO_DRAFT);
    this.running.set(false);
    this.failed.set(null);
    await this.readAll(thread);
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
