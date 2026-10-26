import { ChangeDetectionStrategy, Component, LOCALE_ID, computed, inject } from '@angular/core';
import {
  GlzButtonComponent,
  GlzIconButtonComponent,
  GlzIconComponent,
  type GlzIconData,
  GlzPlanCardComponent,
  injectId,
} from '@glave/glaze';
import { File, Folder, GitBranch, Globe, Laptop, X } from 'lucide';

import { lineCounts } from '../../lib/files';
import { modes } from '../../lib/modes';
import { plural } from '../../lib/plural';
import { spent } from '../../lib/results';
import { PanelService } from '../../services/panel.service';
import { ThreadPanelService } from '../../services/thread-panel.service';
import { CommandListComponent } from '../command-list/command-list.component';

/** One part of the line saying where the thread works. */
interface Part {
  readonly key: string;
  readonly icon: GlzIconData;
  /** What it is, for screen readers. */
  readonly label: string;
  readonly text: string;
  /** All of it, on hover, when the line shows only some. */
  readonly title: string | null;
}

/** How full the context is: its percent, and in words. */
interface Fullness {
  readonly percent: number;
  readonly used: string;
}

/**
 * One column of the open thread's facts, beside it: a line saying where it works (the computer,
 * the folder, the branch, the mode), then each section only when it has something: its plan, in
 * the server's sentence, its steps ticking off (the conversation's card is where it is built);
 * the files it changed with the lines added and removed, each opening its changes in full; its commands
 * running in the background, each with Stop; the pages its replies drew on, each opening in a
 * new tab, and the files the person attached; and how full its context is and what it has
 * cost. As a sheet on a narrow screen it has a heading and a close button; beside the page, its
 * edge and the thread's header hide it.
 */
@Component({
  selector: 'app-thread-panel',
  imports: [
    CommandListComponent,
    GlzButtonComponent,
    GlzIconButtonComponent,
    GlzIconComponent,
    GlzPlanCardComponent,
  ],
  templateUrl: './thread-panel.component.html',
  styleUrl: './thread-panel.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
  host: { '[style.--app-context-used]': 'used()' },
})
export class ThreadPanelComponent {
  protected readonly copy = {
    heading: $localize`:@@threadPanel.heading:This thread`,
    close: $localize`:@@threadPanel.close:Close the panel`,
    where: $localize`:@@threadPanel.where:Where it works`,
    computer: $localize`:@@threadPanel.computer:Computer`,
    folder: $localize`:@@threadPanel.folder:Folder`,
    branch: $localize`:@@threadPanel.branch:Branch`,
    mode: $localize`:@@threadPanel.mode:Mode`,
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
  protected readonly icons = { close: X, page: Globe, file: File } as const;
  protected readonly ids = {
    changes: injectId('app-thread-panel-changes'),
    running: injectId('app-thread-panel-running'),
    sources: injectId('app-thread-panel-sources'),
    spent: injectId('app-thread-panel-spent'),
  } as const;
  private readonly panel = inject(PanelService);
  private readonly facts = inject(ThreadPanelService);
  private readonly locale = inject(LOCALE_ID);
  private readonly numbers = new Intl.NumberFormat(this.locale);
  /** Whether the panel is a sheet over the page, which has a close button of its own. */
  protected readonly narrow = this.panel.narrow;

  protected readonly where = computed<readonly Part[] | null>(() => {
    const where = this.facts.where();
    if (where === null) {
      return null;
    }
    const mode = modes()[where.mode];
    const part = (
      key: string,
      icon: GlzIconData,
      label: string,
      text: string | null,
      title: string | null = null,
    ): readonly Part[] => (text === null ? [] : [{ key, icon, label, text, title }]);
    return [
      ...part('computer', Laptop, this.copy.computer, where.computer),
      ...part('folder', Folder, this.copy.folder, where.folder, where.path),
      ...part('branch', GitBranch, this.copy.branch, where.branch),
      ...part('mode', mode.icon, this.copy.mode, mode.text),
    ];
  });
  /** Its plan, as the server says it. */
  protected readonly plan = this.facts.plan;
  protected readonly changes = computed(() => {
    const files = this.facts.files();
    if (files.length === 0) {
      return null;
    }
    const count = files.length;
    return {
      count: plural(
        $localize`:@@threadPanel.files:{${count}:COUNT:, plural, one {# file} other {# files}}`,
        this.locale,
      ),
      total: lineCounts(
        files.reduce((sum, file) => sum + file.added, 0),
        files.reduce((sum, file) => sum + file.removed, 0),
        this.locale,
      ),
      rows: files.map((file) => ({
        path: file.path,
        ...lineCounts(file.added, file.removed, this.locale),
      })),
    };
  });
  protected readonly running = computed(() => {
    const count = this.facts.running();
    return count === 0 ? null : this.numbers.format(count);
  });
  protected readonly sources = computed(() => {
    const pages = this.facts.pages();
    const attached = this.facts.attached();
    return pages.length === 0 && attached.length === 0 ? null : { pages, attached };
  });
  protected readonly spent = computed(() => {
    const facts = this.facts.spent();
    if (facts === null) {
      return null;
    }
    const turns = facts.turns;
    return {
      context: this.contextOf(facts.context?.percent ?? null),
      cost: [
        spent(facts, this.locale),
        plural(
          $localize`:@@threadPanel.turns:{${turns}:COUNT:, plural, one {# turn} other {# turns}}`,
          this.locale,
        ),
      ].join(this.copy.between),
    };
  });

  /** How much of the context bar is filled, as the styles take it. */
  protected readonly used = computed(() => {
    const percent = this.spent()?.context?.percent;
    return percent === undefined ? null : `${String(percent)}%`;
  });

  protected close(): void {
    this.panel.open.set(false);
  }

  /** Shows the changes in full: the file at `path`, or every file's with `null`. */
  protected review(path: string | null): void {
    this.facts.showChanges(path);
  }

  /** How full the context is, `percent` of it, in words; `null` when the server did not say. */
  private contextOf(percent: number | null): Fullness | null {
    if (percent === null) {
      return null;
    }
    const used = new Intl.NumberFormat(this.locale, { style: 'percent' }).format(percent / 100);
    return { percent, used: $localize`:@@threadPanel.used:${used}:PERCENT: used` };
  }
}
