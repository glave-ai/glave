import { ChangeDetectionStrategy, Component, LOCALE_ID, computed, inject } from '@angular/core';
import { RouterLink } from '@angular/router';
import type { ChangedFile } from '@glave/client';
import {
  GlzButtonComponent,
  GlzCountDirective,
  GlzIconComponent,
  type GlzIconData,
  GlzPlanCardComponent,
} from '@glave/glaze';
import { Folder, GitBranch, Laptop } from 'lucide';

import { lineCounts, lineMarks, totalLines } from '../../lib/files';
import { plural } from '../../lib/plural';
import { ProjectsService } from '../../services/projects.service';
import { ThreadPanelService } from '../../services/thread-panel.service';
import { ThreadService } from '../../services/thread.service';
import { CommandListComponent } from '../command-list/command-list.component';

/** Where the Connections page is. */
const CONNECTIONS = '/settings/connections';

/** What files changed: lines added and removed, in figures and words, and how many files. */
interface Changed {
  readonly added: number;
  readonly removed: number;
  readonly label: string;
  readonly files: string;
}

/** One line of where the thread works: a fact, and what it adds, quieter. */
interface Where {
  readonly key: string;
  readonly icon: GlzIconData;
  /** What it is, for screen readers. */
  readonly label: string;
  readonly text: string;
  readonly detail: string | null;
  /** Whether `detail` is a path, which reads left to right in any page. */
  readonly path: boolean;
}

/**
 * The open thread at a glance, the panel's first tab: each section only when it has something
 * to say. Where it works (the computer, the project and its folder, a worktree's branch, which
 * is a separate copy, or the point a clone is at); its plan, in the server's sentence, its steps
 * ticking off (the conversation's card is where a proposal is built); how full its context is,
 * with Compact now, which waits while a turn runs and shows the server's words when it is
 * refused; how many of the apps its project's threads use need signing in again, which leads to
 * Connections; how many lines and files it changed, their room kept as the thread opens until
 * they are read; and its commands running in the background, each with Stop, shown once what it
 * changed is read when no room was kept for that, so it never moves when that comes in above.
 * Its sections are headed a level below the panel's own heading.
 */
@Component({
  selector: 'app-thread-overview',
  imports: [
    CommandListComponent,
    GlzButtonComponent,
    GlzCountDirective,
    GlzIconComponent,
    GlzPlanCardComponent,
    RouterLink,
  ],
  templateUrl: './thread-overview.component.html',
  styleUrl: './thread-overview.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class ThreadOverviewComponent {
  protected readonly copy = {
    where: $localize`:@@threadOverview.where:Where it works`,
    plan: $localize`:@@threadOverview.plan:Plan`,
    context: $localize`:@@threadOverview.context:Context`,
    connections: $localize`:@@threadOverview.connections:Connections`,
    changes: $localize`:@@threadOverview.changes:Changes`,
    running: $localize`:@@threadOverview.running:Running in the background`,
    between: '·',
  } as const;
  protected readonly facts = inject(ThreadPanelService);
  private readonly thread = inject(ThreadService);
  private readonly projects = inject(ProjectsService);
  private readonly locale = inject(LOCALE_ID);
  /** How lines added and removed are written, as each count counts. */
  protected readonly marks = lineMarks(this.locale);

  /**
   * Whether the facts are kept from sight, their room kept too, while the thread's project isn't
   * known: its name is a line of where it works, so nothing moves if they come in together.
   */
  protected readonly reading = computed(() => {
    const summary = this.thread.summary();
    return (
      summary !== null &&
      this.projects.project(summary.project) === undefined &&
      !this.projects.loaded()
    );
  });
  /** Where it works, as its summary says, a line for each fact. */
  protected readonly where = computed((): readonly Where[] | null => {
    const workspace = this.facts.workspace();
    const summary = this.thread.summary();
    const computer = this.thread.place();
    if (workspace === null || summary === null || computer === null) {
      return null;
    }
    const opened = workspace.opened;
    const project = this.projects.project(summary.project);
    const line = (
      key: string,
      icon: GlzIconData,
      label: string,
      text: string | null,
      detail: string | null = null,
      path = false,
    ): readonly Where[] => (text === null ? [] : [{ key, icon, label, text, detail, path }]);
    return [
      ...line('computer', Laptop, $localize`:@@threadOverview.computer:Computer`, computer),
      // The project's own name, as the server names it, and the folder it has.
      ...line(
        'project',
        Folder,
        $localize`:@@threadOverview.project:Project`,
        project?.kind === 'project' ? project.name : null,
        opened.kind === 'worktree' ? opened.repository : workspace.root,
        true,
      ),
      ...line(
        'branch',
        GitBranch,
        $localize`:@@threadOverview.branch:Branch`,
        opened.kind === 'worktree' ? opened.branch : null,
        $localize`:@@threadOverview.separateCopy:a separate copy`,
      ),
    ];
  });
  /** How full its context is, once a model call says, and compacting it now. */
  protected readonly context = computed(() => {
    const percent = this.thread.summary()?.context?.percent ?? null;
    if (percent === null) {
      return null;
    }
    const used = new Intl.NumberFormat(this.locale, { style: 'percent' }).format(percent / 100);
    const compacting = this.facts.compacting();
    // Not while a turn runs, which compacts on its own as it needs to. Drawn unavailable rather
    // than disabled, so a press on it never loses the keyboard's place.
    const waits = compacting || this.thread.busy();
    return {
      percent,
      used: $localize`:@@threadOverview.used:${used}:PERCENT: used`,
      waits: waits ? 'true' : null,
      action: compacting
        ? $localize`:@@threadOverview.compacting:Compacting…`
        : $localize`:@@threadOverview.compact:Compact now`,
      problem: this.facts.compactProblem(),
    };
  });
  /**
   * How many of its project's apps need signing in again, and where that is done: Connections
   * for the project, or for a chat, Connections itself, which names no project.
   */
  protected readonly signIns = computed(() => {
    const count = this.facts.signInAgain();
    const project = this.thread.summary()?.project ?? null;
    if (count === 0 || project === null) {
      return null;
    }
    const named = this.projects.project(project)?.kind === 'project';
    return {
      text: plural(
        $localize`:@@threadOverview.signIn:{${count}:COUNT:, plural, one {# needs signing in again} other {# need signing in again}}`,
        this.locale,
      ),
      link: CONNECTIONS,
      query: named ? { project } : {},
    };
  });
  /** The lines and files it changed, once there are some. */
  protected readonly changes = computed(() => this.changed(this.facts.files()));
  /**
   * Until those are read, the room of the files its turns' ends recorded, nothing counted: drawn
   * unseen apart from them, so the counts show at once when they come, never counting up there.
   */
  protected readonly room = computed(() =>
    this.facts.files().length === 0 ? this.changed(this.facts.room()) : null,
  );

  /** Whether it keeps commands running in the background. */
  protected readonly running = computed(() => this.facts.running() > 0);
  /**
   * Whether those are kept from sight, their room kept too, while what it changed is first read
   * with no room kept for that: its folder can say a command changed files, whose section comes
   * in above them, so they show once it has, never moving.
   */
  protected readonly runningHeld = computed(() => this.facts.unread() && this.room() === null);

  /** Compacts its context now, unless it waits. */
  protected compact(): void {
    if (this.context()?.waits === null) {
      void this.facts.compact();
    }
  }

  /** What `files` changed, in lines and files, or `null` for none. */
  private changed(files: readonly ChangedFile[]): Changed | null {
    if (files.length === 0) {
      return null;
    }
    const count = files.length;
    const { added, removed } = totalLines(files);
    return {
      added,
      removed,
      label: lineCounts(added, removed, this.locale).label,
      files: plural(
        $localize`:@@threadOverview.files:{${count}:COUNT:, plural, one {# file} other {# files}}`,
        this.locale,
      ),
    };
  }
}
