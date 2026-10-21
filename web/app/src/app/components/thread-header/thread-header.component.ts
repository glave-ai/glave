import {
  ChangeDetectionStrategy,
  Component,
  LOCALE_ID,
  computed,
  inject,
  input,
  output,
} from '@angular/core';
import {
  GlzButtonComponent,
  GlzIconButtonComponent,
  GlzIconComponent,
  GlzTooltipDirective,
} from '@glave/glaze';
import { FileDiff, Folder, Menu, PanelRight } from 'lucide';

import { plural } from '../../lib/plural';
import { NavService } from '../../services/nav.service';
import { ToolsPanelService } from '../../services/tools-panel.service';

/**
 * A thread's bar across its top, ruled off from the conversation: its title, which renames it
 * when pressed; a quiet line with its project and where it runs, as far as the app knows; and
 * at the end "Changes" with how many files the thread changed, which opens the tools panel on
 * them. On a phone the title and that line sit in the middle, between the button that opens the
 * side bar and one that opens the tools panel, each a finger's size.
 */
@Component({
  selector: 'app-thread-header',
  imports: [GlzButtonComponent, GlzIconButtonComponent, GlzIconComponent, GlzTooltipDirective],
  templateUrl: './thread-header.component.html',
  styleUrl: './thread-header.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class ThreadHeaderComponent {
  /** The thread's title. */
  readonly title = input.required<string>();
  /** The name of the project it is in; `null` for a chat. */
  readonly project = input<string | null>(null);
  /** Where it runs, in plain words, if known. */
  readonly place = input<string | null>(null);
  /** How many files the thread changed. */
  readonly changes = input(0);
  /** Emitted when the person asks to rename the thread. */
  readonly rename = output();

  protected readonly copy = {
    menu: $localize`:@@threadHeader.menu:Show or hide the side bar`,
    rename: $localize`:@@threadHeader.rename:Rename the thread`,
    changes: $localize`:@@threadHeader.changes:Changes`,
    tools: $localize`:@@threadHeader.tools:Tools`,
    between: ' · ',
  } as const;
  protected readonly icons = { menu: Menu, folder: Folder, tools: PanelRight };
  private readonly nav = inject(NavService);
  private readonly panel = inject(ToolsPanelService);
  private readonly locale = inject(LOCALE_ID);
  /** A phone: the side bar is a drawer, opened from this bar. */
  protected readonly phone = this.nav.narrow;
  protected readonly navOpen = this.nav.open;
  protected readonly navShortcut = this.nav.ariaShortcut;
  protected readonly panelOpen = this.panel.open;
  protected readonly changesShown = this.panel.changesShown;
  protected readonly panelShortcut = this.panel.ariaShortcut;
  protected readonly changed = computed(() => this.changes() > 0);
  protected readonly count = computed(() =>
    new Intl.NumberFormat(this.locale).format(this.changes()),
  );
  /** Beside a wide page with nothing changed, a button opens the tools only where no edge can. */
  protected readonly toolsButton = computed(
    () => !this.phone() && !this.changed() && this.panel.narrow(),
  );
  /** The line under the title: the project and where it runs, as far as they are known. */
  protected readonly meta = computed(() =>
    [this.project(), this.place()].filter((part) => part !== null).join(this.copy.between),
  );
  /** On a phone, the button at the end: the changes when there are some, else the tools. */
  protected readonly phoneButton = computed(() => {
    const count = this.changes();
    return this.changed()
      ? {
          icon: FileDiff,
          label: plural(
            $localize`:@@threadHeader.changesFiles:Changes, {${count}:COUNT:, plural, one {# file} other {# files}}`,
            this.locale,
          ),
        }
      : { icon: PanelRight, label: this.copy.tools };
  });

  protected startRename(): void {
    this.rename.emit();
  }

  protected toggleNav(): void {
    this.nav.toggle();
  }

  protected toggleChanges(): void {
    this.panel.toggleChanges();
  }

  protected togglePanel(): void {
    this.panel.toggle();
  }

  protected phoneTools(): void {
    if (this.changed()) {
      this.panel.toggleChanges();
    } else {
      this.panel.toggle();
    }
  }
}
