import { Service, computed, inject, signal } from '@angular/core';
import type { ContextUse, Money, Opened, Permissions } from '@glave/client';

import { type SourceLink, sourceLinks } from '../lib/sources';
import { CommandsService } from './commands.service';
import { PanelService } from './panel.service';
import { PlanService } from './plan.service';
import { SourcesService } from './sources.service';
import { ThreadService } from './thread.service';

/** Where a thread works, as its summary says: the computer, the folder, the branch, the mode. */
export interface Where {
  /** The computer, named for the reader. */
  readonly computer: string;
  /** The folder's own name, and where it is in full. */
  readonly folder: string | null;
  readonly path: string | null;
  /** The branch, where the server says one: a worktree's own, or the one a clone is at. */
  readonly branch: string | null;
  readonly mode: Permissions;
}

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

/** What the thread has cost, as its summary says, and how full its context is. */
export interface Spent {
  readonly context: ContextUse | null;
  readonly cost: Money | null;
  readonly estimated: boolean;
  readonly turns: number;
}

/** The last part of a path or an address, a repository's without its `.git`. */
function lastPart(path: string): string {
  const parts = path.split(/[/\\]/u).filter((part) => part !== '');
  return (parts.at(-1) ?? path).replace(/\.git$/u, '');
}

/** The folder a workspace opened, and its branch where the server says one; `null` for scratch. */
function folderOf(
  opened: Opened,
): { readonly folder: string; readonly path: string; readonly branch: string | null } | null {
  switch (opened.kind) {
    case 'folder':
      return { folder: lastPart(opened.path), path: opened.path, branch: null };
    case 'worktree':
      return {
        folder: lastPart(opened.repository),
        path: opened.repository,
        branch: opened.branch,
      };
    case 'clone':
      return { folder: lastPart(opened.url), path: opened.url, branch: opened.at ?? null };
    case 'scratch':
      return null;
  }
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
 * the person attached, and what it has cost. Whether it shows anything at all, so a plain chat
 * has no panel, and whether it shows now; and the changes shown in full, one file's or all of them. Each thread view
 * provides its own.
 */
@Service({ autoProvided: false })
export class ThreadPanelService {
  private readonly thread = inject(ThreadService);
  private readonly commands = inject(CommandsService);
  private readonly sources = inject(SourcesService);
  private readonly plans = inject(PlanService);
  private readonly panel = inject(PanelService);
  /** Whether the changes show in full, and whose: a file's path, or `null` for all of them. */
  private readonly review = signal<{ readonly path: string | null } | null>(null);

  readonly where = computed<Where | null>(() => {
    const summary = this.thread.summary();
    const workspace = summary?.workspace ?? null;
    const folder = workspace === null ? null : folderOf(workspace.opened);
    const computer = this.thread.place();
    if (summary === null || folder === null || computer === null) {
      return null;
    }
    return { computer, ...folder, mode: summary.permissions };
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
  /** What it has cost and how full its context is, once it has run a turn. */
  readonly spent = computed<Spent | null>(() => {
    const summary = this.thread.summary();
    const context = summary?.context ?? null;
    if (summary === null || (summary.turns === 0 && context === null)) {
      return null;
    }
    const { turns, cost = null, estimated = false } = summary;
    return { context, cost, estimated, turns };
  });
  /**
   * Whether the panel has anything to show: what it has cost alone is not enough, so a plain
   * chat has no panel.
   */
  readonly shown = computed(
    () =>
      this.where() !== null ||
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
  readonly visible = computed(() => this.shown() && this.panel.open());
  /** Whether the changes show in full. */
  readonly reviewing = computed(() => this.review() !== null);
  /** Whose changes show in full: a file's path, or `null` for every file's. */
  readonly reviewed = computed(() => this.review()?.path ?? null);

  /** Shows the changes in full: the file at `path`, or every file's with `null`. */
  showChanges(path: string | null): void {
    this.review.set({ path });
  }

  /** Shows them in full no longer, once `open` is false: the dialog says so as it closes. */
  setReviewing(open: boolean): void {
    if (!open) {
      this.review.set(null);
    }
  }
}
