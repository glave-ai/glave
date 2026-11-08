import {
  ChangeDetectionStrategy,
  Component,
  DOCUMENT,
  ElementRef,
  afterEveryRender,
  computed,
  inject,
  linkedSignal,
  output,
  viewChild,
} from '@angular/core';
import type { ThreadId } from '@glave/client';
import {
  GlzIconButtonComponent,
  GlzIconComponent,
  GlzTabComponent,
  GlzTabsComponent,
} from '@glave/glaze';
import { X } from 'lucide';

import { PanelService } from '../../services/panel.service';
import { ThreadPanelService } from '../../services/thread-panel.service';
import { ThreadService } from '../../services/thread.service';
import { ThreadFilesComponent } from '../thread-files/thread-files.component';
import { ThreadOverviewComponent } from '../thread-overview/thread-overview.component';
import { ThreadPreviewComponent } from '../thread-preview/thread-preview.component';

/** The panel's tabs, each by what it shows. */
type Tab = 'overview' | 'changes' | 'preview';

/** What the tab chosen follows: the thread, and each ask to show a preview. */
interface Asked {
  readonly thread: ThreadId | null;
  readonly preview: number;
}

/** A tab as the panel shows it. */
interface Shown {
  readonly key: Tab;
  readonly label: string;
}

/** The tab a thread opens on. */
const FIRST: Tab = 'overview';

/**
 * The open thread's panel, beside it: tabs, each only once it has something to show, in this
 * order: Overview, the thread at a glance, which it opens on (`app-thread-overview`); then the
 * files it changed, in full; then, while one of its commands serves something the person can
 * open here, the Preview (`app-thread-preview`), which an Open preview elsewhere in the panel
 * shows. The tab chosen is this view's to remember, and each thread opens on Overview again.
 * Its heading is seen in the sheet of a narrow screen, with a close button, and heard beside
 * the page; its sections are a level below. When what had the keyboard's place goes from it, as
 * Stop takes the last command away or Compact now the context's section, that place goes to the
 * heading, or, when the panel goes, `gone` says so.
 */
@Component({
  selector: 'app-thread-panel',
  imports: [
    GlzIconButtonComponent,
    GlzIconComponent,
    GlzTabComponent,
    GlzTabsComponent,
    ThreadFilesComponent,
    ThreadOverviewComponent,
    ThreadPreviewComponent,
  ],
  templateUrl: './thread-panel.component.html',
  styleUrl: './thread-panel.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
  host: {
    '[class.sheet]': 'narrow()',
    '(focusin)': 'focused($event)',
    '(focusout)': 'left($event)',
  },
})
export class ThreadPanelComponent {
  /** The panel has nothing left to show, and the keyboard's place went with what was there. */
  readonly gone = output();

  protected readonly copy = {
    heading: $localize`:@@threadPanel.heading:This thread`,
    close: $localize`:@@threadPanel.close:Close the panel`,
    tabs: $localize`:@@threadPanel.tabs:What this thread shows`,
    overview: $localize`:@@threadPanel.overview:Overview`,
    changes: $localize`:@@threadPanel.changes:Changes`,
    preview: $localize`:@@threadPanel.preview:Preview`,
  } as const;
  protected readonly icons = { close: X } as const;
  private readonly panel = inject(PanelService);
  protected readonly facts = inject(ThreadPanelService);
  private readonly thread = inject(ThreadService);
  private readonly document = inject(DOCUMENT);
  private readonly host = inject<ElementRef<HTMLElement>>(ElementRef);
  private readonly heading = viewChild.required<ElementRef<HTMLElement>>('heading');
  /** Whether the panel is a sheet over the page, which shows its heading and a close button. */
  protected readonly narrow = this.panel.narrow;
  /** Beside the page, the heading is for screen readers alone. */
  protected readonly beside = computed(() => !this.narrow());
  /** What last had the keyboard's place in the panel. */
  private last: Element | null = null;

  /** The tabs it has now, in order, each with its label. */
  protected readonly tabs = computed((): readonly Shown[] => {
    const changes = this.facts.files().length > 0;
    const preview = this.facts.previews().length > 0;
    return [
      { key: 'overview', label: this.copy.overview },
      ...(changes ? [{ key: 'changes', label: this.copy.changes } as const] : []),
      ...(preview ? [{ key: 'preview', label: this.copy.preview } as const] : []),
    ];
  });
  /**
   * The tab chosen, by what it shows: Overview again in each thread, and the Preview each time
   * a preview is asked to be shown.
   */
  private readonly chosen = linkedSignal<Asked, Tab>({
    source: () => ({ thread: this.thread.thread(), preview: this.facts.previewAsked() }),
    computation: (now, before) => {
      const same = before?.source.thread === now.thread;
      if (same && before.source.preview !== now.preview) {
        return 'preview';
      }
      return same ? before.value : FIRST;
    },
  });
  /** Where the chosen tab is among those it has, or the first's when it has it no longer. */
  protected readonly selected = computed(() =>
    Math.max(
      0,
      this.tabs().findIndex((tab) => tab.key === this.chosen()),
    ),
  );

  constructor() {
    // What had the keyboard's place went from the panel as it drew: the place goes to its
    // heading, or, when the panel has nothing left, to whoever holds the panel.
    afterEveryRender(() => {
      const lost =
        this.last !== null &&
        !this.last.isConnected &&
        this.document.activeElement === this.document.body;
      if (!lost) {
        return;
      }
      this.last = null;
      if (this.facts.shown()) {
        this.heading().nativeElement.focus();
      } else {
        this.gone.emit();
      }
    });
  }

  protected focused(event: FocusEvent): void {
    this.last = event.target instanceof Element ? event.target : null;
  }

  /** The keyboard's place moved on to something outside the panel: nothing of it is lost. */
  protected left(event: FocusEvent): void {
    const to = event.relatedTarget;
    if (to instanceof Node && !this.host.nativeElement.contains(to)) {
      this.last = null;
    }
  }

  /**
   * Shows the tab at `index`. What had the keyboard's place in the tab before goes with it on
   * purpose: the place stays where the person pressed, never moved to the heading.
   */
  protected choose(index: number): void {
    const tab = this.tabs()[index];
    if (tab !== undefined) {
      this.last = null;
      this.chosen.set(tab.key);
    }
  }

  protected close(): void {
    this.panel.setShowing(false);
  }
}
