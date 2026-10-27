import { Service, computed, effect, inject, signal, untracked } from '@angular/core';
import type { WorkspaceBound } from '@glave/client';

import type { ChangedFile } from '../lib/files';
import { type SourceLink, sourceLinks } from '../lib/sources';
import { CommandsService } from './commands.service';
import { PanelService } from './panel.service';
import { PlanService } from './plan.service';
import { SourcesService } from './sources.service';
import { ThreadService } from './thread.service';

/** A page a reply drew on: its title, else where it is, and the site it is on. */
export interface Page extends SourceLink {
  readonly name: string;
  readonly host: string;
}

/** A file the person attached to a message. */
export interface Attached {
  readonly key: string;
  readonly name: string;
}

/** Changes shown in full: a turn's files or, `null`, the thread's; one file's, or every one's. */
interface Review {
  readonly files: readonly ChangedFile[] | null;
  readonly path: string | null;
}

function hostOf(link: SourceLink): string {
  try {
    return new URL(link.href).host;
  } catch {
    return link.address;
  }
}

/**
 * What the open thread's panel shows, each from the server's facts: where it works (a folder
 * of a project; a chat's scratch folder is nowhere the person chose), its plan, the files it
 * changed, its commands running in the background, the pages its replies drew on and the files
 * the person attached. Whether it shows anything at all, so a plain chat has no panel, and
 * whether it shows now; a panel left with nothing closes its sheet. And the changes shown in
 * full: a turn's or the thread's, one file's or all of them. Each thread view provides its own.
 */
@Service({ autoProvided: false })
export class ThreadPanelService {
  private readonly thread = inject(ThreadService);
  private readonly commands = inject(CommandsService);
  private readonly sources = inject(SourcesService);
  private readonly plans = inject(PlanService);
  private readonly panel = inject(PanelService);
  private readonly review = signal<Review | null>(null);

  /** Where it works, once its summary says: a folder the person chose, not a scratch one. */
  readonly workspace = computed<WorkspaceBound | null>(() => {
    const workspace = this.thread.summary()?.workspace ?? null;
    return workspace?.opened.kind === 'scratch' ? null : workspace;
  });
  /** Its plan, as the server reads it. */
  readonly plan = this.plans.plan;
  readonly files = this.thread.files;
  readonly running = this.commands.count;
  /** The pages its replies drew on, each once, in the order first read. */
  readonly pages = computed<readonly Page[]>(() => {
    const links = [...this.sources.byReply().values()].flatMap(sourceLinks);
    const seen = new Set<string>();
    return links.flatMap((link) => {
      if (seen.has(link.href)) {
        return [];
      }
      seen.add(link.href);
      return [{ ...link, name: link.title ?? link.address, host: hostOf(link) }];
    });
  });
  /** The files the person attached to their messages, in the order they sent them. */
  readonly attached = computed<readonly Attached[]>(() =>
    this.thread
      .turns()
      .flatMap((turn) => turn.items)
      .flatMap((item) =>
        item.kind === 'message' && item.from === 'person'
          ? item.files.map((file, index) => ({
              key: `${item.key}-${String(index)}`,
              name: file.name,
            }))
          : [],
      ),
  );
  /**
   * Whether the panel has anything to show: what it has cost alone is not enough, so a plain
   * chat has no panel.
   */
  readonly shown = computed(
    () =>
      this.workspace() !== null ||
      this.plan() !== null ||
      this.files().length > 0 ||
      this.running() > 0 ||
      this.pages().length > 0 ||
      this.attached().length > 0,
  );
  /**
   * Whether the panel shows now: the thread has one, and it is open, beside the page or, on a
   * narrow screen, as a sheet over it.
   */
  readonly visible = computed(() => this.shown() && this.panel.showing());
  /** Whether the changes show in full. */
  readonly reviewing = computed(() => this.review() !== null);
  /** The files whose changes show in full: a turn's, else the thread's. */
  readonly reviewedFiles = computed(() => this.review()?.files ?? this.files());
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
  }

  /**
   * Shows changes in full: of `files`, a turn's, or the thread's with `null`; the file at `path`,
   * or every one's with `null`.
   */
  showChanges(files: readonly ChangedFile[] | null, path: string | null): void {
    this.review.set({ files, path });
  }

  /** Shows them in full no longer, once `open` is false: the dialog says so as it closes. */
  setReviewing(open: boolean): void {
    if (!open) {
      this.review.set(null);
    }
  }

  /** Starts another thread afresh: no changes in full, its sheet closed. */
  reset(): void {
    this.review.set(null);
    this.panel.closeSheet();
  }
}
