import {
  ChangeDetectionStrategy,
  Component,
  DOCUMENT,
  type ElementRef,
  LOCALE_ID,
  afterRenderEffect,
  computed,
  inject,
  output,
  viewChild,
} from '@angular/core';
import {
  GlzButtonComponent,
  GlzCardComponent,
  GlzIconButtonComponent,
  GlzIconComponent,
  type GlzIconData,
  GlzPlanCardComponent,
} from '@glave/glaze';
import { File, Folder, GitBranch, GitCommitHorizontal, Globe, Laptop, X } from 'lucide';

import { modes } from '../../lib/modes';
import { plural } from '../../lib/plural';
import { spent } from '../../lib/results';
import { PanelService } from '../../services/panel.service';
import { ProjectsService } from '../../services/projects.service';
import { ThreadPanelService } from '../../services/thread-panel.service';
import { ThreadService } from '../../services/thread.service';
import { CommandListComponent } from '../command-list/command-list.component';
import { PanelThoughtComponent } from '../panel-thought/panel-thought.component';
import { TurnChangesComponent } from '../turn-changes/turn-changes.component';

/** One part of the line saying where the thread works. */
interface Part {
  readonly key: string;
  readonly icon: GlzIconData;
  /** What it is, for screen readers. */
  readonly label: string;
  readonly text: string;
}

/**
 * One column of the open thread's facts, beside it: a line saying where it works (the computer,
 * the project's folder, its full path a press away, a worktree's branch or the point a clone is
 * at, the mode), then each section only when it has something: the model's reasoning the
 * person chose to read in full, from its line in the conversation; its plan, in the server's
 * sentence, its steps ticking off (the conversation's card is where a proposal is built); the
 * files it changed, as a turn's card shows them, each opening its changes in full, their room
 * kept as the thread opens until they are read; its commands running in the background, each
 * with Stop; the pages its replies drew on, each opening in a new tab, and the files the person
 * attached; and how full its context is and what it has cost.
 * Its heading is seen in the sheet of a narrow screen, with a close button, and heard beside
 * the page; its sections are a level below. When Stop takes the last command away with the
 * keyboard's place, that place goes to the heading, or, when the panel goes, `gone` says so.
 */
@Component({
  selector: 'app-thread-panel',
  imports: [
    CommandListComponent,
    GlzButtonComponent,
    GlzCardComponent,
    GlzIconButtonComponent,
    GlzIconComponent,
    GlzPlanCardComponent,
    PanelThoughtComponent,
    TurnChangesComponent,
  ],
  templateUrl: './thread-panel.component.html',
  styleUrl: './thread-panel.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
  host: { '[class.sheet]': 'narrow()' },
})
export class ThreadPanelComponent {
  /** The panel has nothing left to show, and the keyboard's place went with what was there. */
  readonly gone = output();

  protected readonly copy = {
    heading: $localize`:@@threadPanel.heading:This thread`,
    close: $localize`:@@threadPanel.close:Close the panel`,
    where: $localize`:@@threadPanel.where:Where it works`,
    computer: $localize`:@@threadPanel.computer:Computer`,
    folder: $localize`:@@threadPanel.folder:Folder`,
    branch: $localize`:@@threadPanel.branch:Branch`,
    at: $localize`:@@threadPanel.at:Checked out at`,
    mode: $localize`:@@threadPanel.mode:Mode`,
    plan: $localize`:@@threadPanel.plan:Plan`,
    changes: $localize`:@@threadPanel.changes:Changes`,
    review: $localize`:@@threadPanel.review:Review all changes`,
    running: $localize`:@@threadPanel.running:Running`,
    sources: $localize`:@@threadPanel.sources:Sources and files`,
    newTab: $localize`:@@threadPanel.newTab:(opens in a new tab)`,
    attached: $localize`:@@threadPanel.attached:you attached`,
    spent: $localize`:@@threadPanel.spent:Context and cost`,
    context: $localize`:@@threadPanel.context:Context`,
    cost: $localize`:@@threadPanel.cost:Cost so far`,
    between: ' · ',
  } as const;
  protected readonly icons = { close: X, folder: Folder, page: Globe, file: File } as const;
  private readonly panel = inject(PanelService);
  protected readonly facts = inject(ThreadPanelService);
  private readonly thread = inject(ThreadService);
  private readonly projects = inject(ProjectsService);
  private readonly locale = inject(LOCALE_ID);
  private readonly document = inject(DOCUMENT);
  private readonly heading = viewChild.required<ElementRef<HTMLElement>>('heading');
  /** Whether the panel is a sheet over the page, which shows its heading and a close button. */
  protected readonly narrow = this.panel.narrow;
  /** Beside the page, the heading is for screen readers alone. */
  protected readonly beside = computed(() => !this.narrow());
  /** Whether the last command was there as the panel last drew, so its going can be seen. */
  private ran = false;

  /** Where it works, as its summary says, in one line: the folder apart, a press away. */
  protected readonly where = computed(() => {
    const workspace = this.facts.workspace();
    const summary = this.thread.summary();
    const computer = this.thread.place();
    if (workspace === null || summary === null || computer === null) {
      return null;
    }
    const opened = workspace.opened;
    const project = this.projects.project(summary.project);
    const mode = modes()[summary.permissions];
    const part = (
      key: string,
      icon: GlzIconData,
      label: string,
      text: string | null,
    ): readonly Part[] => (text === null ? [] : [{ key, icon, label, text }]);
    return {
      // The project's own name, as the server names it, and where its tools act in full.
      folder: project?.kind === 'project' ? { name: project.name, path: workspace.root } : null,
      before: part('computer', Laptop, this.copy.computer, computer),
      after: [
        ...part(
          'branch',
          GitBranch,
          this.copy.branch,
          opened.kind === 'worktree' ? opened.branch : null,
        ),
        ...part(
          'at',
          GitCommitHorizontal,
          this.copy.at,
          opened.kind === 'clone' ? (opened.at ?? null) : null,
        ),
        ...part('mode', mode.icon, this.copy.mode, mode.text),
      ],
    };
  });
  /** The files it changed, once there are some. */
  protected readonly changes = computed(() => {
    const files = this.facts.files();
    return files.length === 0 ? null : files;
  });
  /** Until those are known, the files whose room is kept, unseen; else `null`. */
  protected readonly room = computed(() => {
    const room = this.facts.room();
    return room.length === 0 ? null : room;
  });
  protected readonly running = computed(() => {
    const count = this.facts.running();
    return count === 0 ? null : new Intl.NumberFormat(this.locale).format(count);
  });
  protected readonly sources = computed(() => {
    const pages = this.facts.pages();
    const attached = this.facts.attached();
    return pages.length === 0 && attached.length === 0 ? null : { pages, attached };
  });
  /** How full its context is and what it has cost, once it has run a turn. */
  protected readonly spent = computed(() => {
    const summary = this.thread.summary();
    const percent = summary?.context?.percent ?? null;
    if (summary === null || (summary.turns === 0 && percent === null)) {
      return null;
    }
    const turns = summary.turns;
    const used = new Intl.NumberFormat(this.locale, { style: 'percent' }).format(
      (percent ?? 0) / 100,
    );
    return {
      context:
        percent === null
          ? null
          : { percent, used: $localize`:@@threadPanel.used:${used}:PERCENT: used` },
      cost: [
        spent({ cost: summary.cost ?? null, estimated: summary.estimated ?? false }, this.locale),
        plural(
          $localize`:@@threadPanel.turns:{${turns}:COUNT:, plural, one {# turn} other {# turns}}`,
          this.locale,
        ),
      ].join(this.copy.between),
    };
  });

  constructor() {
    // Stop takes the last command away, and the keyboard's place with it: the place goes to the
    // panel's heading, or, when the panel has nothing left, to whoever holds the panel.
    afterRenderEffect(() => {
      const runs = this.facts.running() > 0;
      const lost = this.ran && !runs && this.document.activeElement === this.document.body;
      this.ran = runs;
      if (!lost) {
        return;
      }
      if (this.facts.shown()) {
        this.heading().nativeElement.focus();
      } else {
        this.gone.emit();
      }
    });
  }

  protected close(): void {
    this.panel.setShowing(false);
  }

  /** Shows every change the thread made in full. */
  protected reviewAll(): void {
    this.facts.showChanges(null, null);
  }
}
