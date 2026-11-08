import { Service, computed, effect, inject, linkedSignal, signal, untracked } from '@angular/core';
import {
  ApiError,
  type ChangedFile,
  type ProjectId,
  type ThreadId,
  type WorkspaceBound,
} from '@glave/client';

import { type Preview, previewKey } from '../lib/previews';
import { sentence, unreached } from '../lib/text';
import { onChange } from './changes.service';
import { ChangedFilesService } from './changed-files.service';
import { CommandsService } from './commands.service';
import { PanelService } from './panel.service';
import { PlanService } from './plan.service';
import { SessionService } from './session.service';
import { ThreadService } from './thread.service';

/** Changes shown in full: a turn's files, one file's or every one's. */
interface Review {
  readonly files: readonly ChangedFile[];
  readonly path: string | null;
}

/** How many of the apps a project's threads use need signing in again, as last listed. */
interface SignIns {
  readonly project: ProjectId;
  readonly again: number;
}

/** Compacting the thread's context on request: under way, or why it was refused. */
interface Compacting {
  readonly thread: ThreadId | null;
  readonly busy: boolean;
  readonly problem: string | null;
}

/**
 * What the open thread's panel shows, each from the server's facts: where it works (a folder
 * of a project; a chat's scratch folder is nowhere the person chose), its plan, the files it
 * changed, its commands running in the background, and how many of the apps its project's
 * threads use need signing in again (`connector/list`, read as its project is known and again
 * as the person's connections change), and the previews of what those commands serve, the one
 * shown the person's choice. Whether it shows anything at all, so a plain chat has no
 * panel, and whether it shows now; a panel left with nothing closes its sheet. Compacting its
 * context on request (`thread/compact`), a refusal in the server's words. And a turn's changes
 * shown in full, one file's or all of them. Each thread view provides its own.
 */
@Service({ autoProvided: false })
export class ThreadPanelService {
  private readonly session = inject(SessionService);
  private readonly thread = inject(ThreadService);
  private readonly commands = inject(CommandsService);
  private readonly changed = inject(ChangedFilesService);
  private readonly plans = inject(PlanService);
  private readonly panel = inject(PanelService);
  private readonly review = signal<Review | null>(null);
  private readonly signIns = signal<SignIns | null>(null);
  /** Starts again from nothing in each thread. */
  private readonly compaction = linkedSignal<ThreadId | null, Compacting>({
    source: this.thread.thread,
    computation: (thread) => ({ thread, busy: false, problem: null }),
  });
  /** Counts the reads of the sign-ins begun, so an answer a later one overtook is dropped. */
  private asked = 0;
  /** The thread's project, once its summary says. */
  private readonly project = computed(() => this.thread.summary()?.project ?? null);

  /** Where it works, once its summary says: a folder the person chose, not a scratch one. */
  readonly workspace = computed<WorkspaceBound | null>(() => {
    const workspace = this.thread.summary()?.workspace ?? null;
    return workspace?.opened.kind === 'scratch' ? null : workspace;
  });
  /** Its plan, as the server reads it. */
  readonly plan = this.plans.plan;
  /** The files it changed, as the server says. */
  readonly files = this.changed.files;
  /** Until those are known, the files whose room is kept, nothing counted. */
  readonly room = this.changed.room;
  /** Whether those are yet to be first read. */
  readonly unread = this.changed.unread;
  /** What those leave out, in the server's sentence. */
  readonly note = this.changed.note;
  readonly running = this.commands.count;
  /** The previews of what its commands serve, as the server opened them for this person. */
  readonly previews = this.commands.previews;
  /** The preview the person chose last, by its key: the first is shown until they choose. */
  private readonly choice = linkedSignal<ThreadId | null, string | null>({
    source: this.thread.thread,
    computation: () => null,
  });
  /** The preview shown: the one chosen while it serves, else the first; none while none does. */
  readonly preview = computed(() => {
    const previews = this.previews();
    const chosen = this.choice();
    return previews.find((each) => previewKey(each) === chosen) ?? previews[0] ?? null;
  });
  /** How many times a preview was asked to be shown in its tab: each ask shows the tab. */
  private readonly previewAsks = signal(0);
  readonly previewAsked = this.previewAsks.asReadonly();
  /**
   * How many of the apps its project's threads use need signing in again: none until listed,
   * or when they could not be.
   */
  readonly signInAgain = computed(() => {
    const listed = this.signIns();
    return listed !== null && listed.project === this.project() ? listed.again : 0;
  });
  /** Whether its context is being compacted on request. */
  readonly compacting = computed(() => this.compaction().busy);
  /** Why it could not be compacted last, in the server's words. */
  readonly compactProblem = computed(() => this.compaction().problem);
  /**
   * Whether the panel has anything to show: how full its context is alone is not enough, so a
   * plain chat has no panel.
   */
  readonly shown = computed(
    () =>
      this.workspace() !== null ||
      this.plan() !== null ||
      this.files().length > 0 ||
      this.room().length > 0 ||
      this.running() > 0,
  );
  /**
   * Whether the panel shows now: the thread has one, and it is open, beside the page or, on a
   * narrow screen, as a sheet over it.
   */
  readonly visible = computed(() => this.shown() && this.panel.showing());
  /**
   * Whether it is known if there is a panel, so the page can be drawn at its shape: once the
   * thread's summary is (from the side nav's lists, else its own read), or its first message was
   * just sent here, before it has any facts; or it could not be read, which the page says.
   */
  readonly decided = computed(
    () =>
      this.thread.summary() !== null ||
      this.thread.firstTurn() !== null ||
      this.thread.error() !== null,
  );
  /** Whether the thread has been read as it opened, kept per thread: `read`. */
  private readonly settled = linkedSignal<
    { readonly thread: ThreadId | null; readonly ready: boolean },
    boolean
  >({
    source: () => ({
      thread: this.thread.thread(),
      ready: this.thread.firstTurn() !== null || !this.thread.opening(),
    }),
    computation: (now, before) =>
      now.ready || (before?.source.thread === now.thread && before.value),
  });
  /**
   * Whether the thread's history has been read as it opened, so the conversation is drawn from
   * it; at once for a thread whose first message was just sent here, which has none. Nothing
   * else is waited for: what it changed, its plan, its commands and its project's sign-ins each
   * come into their own place as they are read, so a read that is slow, or never answers, holds
   * back only what it brings. Once read, it stays so in the thread, whatever is
   * read again as the person watches.
   */
  readonly read = this.settled.asReadonly();
  /** Whether the changes show in full. */
  readonly reviewing = computed(() => this.review() !== null);
  /** The files whose changes show in full: a turn's. */
  readonly reviewedFiles = computed(() => this.review()?.files ?? []);
  /** Whose changes show in full: a file's path, or `null` for every file's. */
  readonly reviewed = computed(() => this.review()?.path ?? null);

  constructor() {
    // A panel left with nothing to show closes its sheet, so it never comes back over the page.
    effect(() => {
      if (!this.shown()) {
        untracked(() => {
          this.panel.closeSheet();
        });
      }
    });
    // Its project's sign-ins, read as soon as its project is known: from the side nav's lists,
    // before its history is, so they are there as the panel is drawn.
    effect(() => {
      const project = this.project();
      if (project !== null) {
        untracked(() => void this.readSignIns(project));
      }
    });
    onChange('connections', () => {
      const project = this.project();
      if (project !== null) {
        void this.readSignIns(project);
      }
    });
  }

  /** Shows a turn's changes in full, of `files`: the file at `path`, or every one's with `null`. */
  showChanges(files: readonly ChangedFile[], path: string | null): void {
    this.review.set({ files, path });
  }

  /**
   * Compacts the thread's context now: the server answers once it has, and its summary says how
   * full the context is from then on; a refusal shows in the server's words.
   */
  async compact(): Promise<void> {
    const thread = this.thread.thread();
    if (thread === null || this.compacting()) {
      return;
    }
    this.compaction.set({ thread, busy: true, problem: null });
    let problem: string | null = null;
    try {
      await this.session.call('thread/compact', { thread });
    } catch (error: unknown) {
      problem = error instanceof ApiError ? sentence(error.message) : unreached();
    }
    // Unless another thread opened meanwhile, which starts afresh.
    this.compaction.update((state) =>
      state.thread === thread ? { thread, busy: false, problem } : state,
    );
  }

  /** Shows them in full no longer, once `open` is false: the dialog says so as it closes. */
  setReviewing(open: boolean): void {
    if (!open) {
      this.review.set(null);
    }
  }

  /** Lists the apps `project`'s threads use, and counts those that need signing in again. */
  private async readSignIns(project: ProjectId): Promise<void> {
    this.asked += 1;
    const asked = this.asked;
    try {
      const { connectors } = await this.session.call('connector/list', { used_in: project });
      const again = connectors.filter((each) => each.status === 'sign_in_again').length;
      if (asked === this.asked) {
        this.signIns.set({ project, again });
      }
    } catch {
      // Unread, it says nothing: the Connections page says why when the person opens it.
    }
  }

  /** Shows the preview whose key is `key` in the Preview tab, from now on. */
  choosePreview(key: string): void {
    this.choice.set(key);
  }

  /** Shows `preview` in the Preview tab, and asks the panel to show that tab. */
  showPreview(preview: Preview): void {
    this.choice.set(previewKey(preview));
    this.previewAsks.update((asks) => asks + 1);
  }

  /** Starts another thread afresh: no changes in full, its sheet closed. */
  reset(): void {
    this.review.set(null);
    this.panel.closeSheet();
  }
}
