import {
  ChangeDetectionStrategy,
  Component,
  booleanAttribute,
  computed,
  inject,
  input,
  output,
} from '@angular/core';
import { GlzIconButtonComponent, GlzIconComponent, GlzTooltipDirective } from '@glave/glaze';
import { Folder, Menu, PanelRight } from 'lucide';

import { NavService } from '../../services/nav.service';
import { PanelService } from '../../services/panel.service';

/**
 * A thread's bar across its top, ruled off from the conversation: its title, which renames it
 * when pressed; a quiet line with its project; and at the end, when the thread has a panel of
 * facts, the button that shows or hides it: a toggle, pressed while it shows, beside a wide
 * page; on a narrower one, where it opens a sheet over the page, a button that says whether the
 * sheet is expanded. On a phone the title and that line sit in the
 * middle, between the button that opens the side bar and the panel's, each a finger's size.
 */
@Component({
  selector: 'app-thread-header',
  imports: [GlzIconButtonComponent, GlzIconComponent, GlzTooltipDirective],
  templateUrl: './thread-header.component.html',
  styleUrl: './thread-header.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class ThreadHeaderComponent {
  /** The thread's title. */
  readonly title = input.required<string>();
  /** The name of the project it is in; `null` for a chat. */
  readonly project = input<string | null>(null);
  /** Whether the thread has a panel of facts to show or hide. */
  readonly panel = input(false, { transform: booleanAttribute });
  /** Emitted when the person asks to rename the thread. */
  readonly rename = output();

  protected readonly copy = {
    menu: $localize`:@@threadHeader.menu:Show or hide the side bar`,
    rename: $localize`:@@threadHeader.rename:Rename the thread`,
    panel: $localize`:@@threadHeader.panel:Show or hide the thread panel`,
  } as const;
  protected readonly icons = { menu: Menu, folder: Folder, panel: PanelRight };
  private readonly nav = inject(NavService);
  private readonly panels = inject(PanelService);
  /** A phone: the side bar is a drawer, opened from this bar. */
  protected readonly phone = this.nav.narrow;
  protected readonly navOpen = this.nav.open;
  protected readonly navShortcut = this.nav.ariaShortcut;
  protected readonly panelShortcut = this.panels.ariaShortcut;
  /** Beside a wide page: whether the toggle is pressed. */
  protected readonly pressed = computed(() =>
    this.panels.narrow() ? null : String(this.panels.showing()),
  );
  /** Over a narrow page: whether the sheet it opens is expanded. */
  protected readonly expanded = computed(() =>
    this.panels.narrow() ? String(this.panels.showing()) : null,
  );

  protected startRename(): void {
    this.rename.emit();
  }

  protected toggleNav(): void {
    this.nav.toggle();
  }

  protected togglePanel(): void {
    this.panels.toggle();
  }
}
