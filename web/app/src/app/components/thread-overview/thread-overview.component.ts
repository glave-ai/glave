import { ChangeDetectionStrategy, Component, LOCALE_ID, computed, inject } from '@angular/core';
import { RouterLink } from '@angular/router';
import {
  GlzButtonComponent,
  GlzCountDirective,
  GlzIconComponent,
  type GlzIconData,
  GlzPlanCardComponent,
} from '@glave/glaze';
import { Folder, GitBranch, GitCommitHorizontal, Laptop } from 'lucide';

import { lineCounts, lineMarks, totalLines } from '../../lib/files';
import { plural } from '../../lib/plural';
import { ProjectsService } from '../../services/projects.service';
import { ThreadPanelService } from '../../services/thread-panel.service';
import { ThreadService } from '../../services/thread.service';
import { CommandListComponent } from '../command-list/command-list.component';

/** Where the Connections page is. */
const CONNECTIONS = '/settings/connections';

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
 * they are read; and its commands running in the background, each with Stop. Its sections are
 * headed a level below the panel's own heading.
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
        $localize`:@@threadOverview.copy:a separate copy`,
      ),
      ...line(
        'at',
        GitCommitHorizontal,
        $localize`:@@threadOverview.at:Checked out at`,
        opened.kind === 'clone' ? (opened.at ?? null) : null,
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
    return {
      percent,
      used: $localize`:@@threadOverview.used:${used}:PERCENT: used`,
      // Not while a turn runs, which compacts on its own as it needs to.
      waits: compacting || this.thread.busy(),
      action: compacting
        ? $localize`:@@threadOverview.compacting:Compacting…`
        : $localize`:@@threadOverview.compact:Compact now`,
      problem: this.facts.compactProblem(),
    };
  });
  /** How many of its project's apps need signing in again, and where that is done. */
  protected readonly signIns = computed(() => {
    const count = this.facts.signInAgain();
    const project = this.thread.summary()?.project ?? null;
    if (count === 0 || project === null) {
      return null;
    }
    return {
      text: plural(
        $localize`:@@threadOverview.signIn:{${count}:COUNT:, plural, one {# needs signing in again} other {# need signing in again}}`,
        this.locale,
      ),
      link: CONNECTIONS,
      query: { project },
    };
  });
  /**
   * The lines and files it changed, once there are some; until they are read, the room of the
   * files its turns' ends recorded, unseen, nothing counted.
   */
  protected readonly changes = computed(() => {
    const files = this.facts.files();
    const room = files.length === 0 ? this.facts.room() : [];
    const shown = files.length === 0 ? room : files;
    if (shown.length === 0) {
      return null;
    }
    const count = shown.length;
    const { added, removed } = totalLines(shown);
    return {
      added,
      removed,
      label: lineCounts(added, removed, this.locale).label,
      files: plural(
        $localize`:@@threadOverview.files:{${count}:COUNT:, plural, one {# file} other {# files}}`,
        this.locale,
      ),
      room: room.length > 0,
    };
  });

  /** Whether it keeps commands running in the background. */
  protected readonly running = computed(() => this.facts.running() > 0);

  /** Compacts its context now. */
  protected compact(): void {
    void this.facts.compact();
  }
}
