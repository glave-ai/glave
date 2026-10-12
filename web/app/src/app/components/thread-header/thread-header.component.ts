import { ChangeDetectionStrategy, Component, inject, input, output } from '@angular/core';
import { GlzBadgeComponent, GlzIconButtonComponent, GlzIconComponent } from '@glave/glaze';
import { PanelRight, Pencil } from 'lucide';

import type { ThreadMark } from '../../lib/threads';
import { ToolsPanelService } from '../../services/tools-panel.service';
import { PageHeaderComponent } from '../page-header/page-header.component';

const COPY = {
  rename: 'Rename the thread',
  tools: 'Tools',
} as const;

/**
 * A thread's top bar, floating over its conversation: the space, the title, its mark and
 * rename; and, on narrow screens, where the tools panel is a sheet, the button that opens it.
 */
@Component({
  selector: 'app-thread-header',
  imports: [GlzBadgeComponent, GlzIconButtonComponent, GlzIconComponent, PageHeaderComponent],
  templateUrl: './thread-header.component.html',
  styleUrl: './thread-header.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class ThreadHeaderComponent {
  /** The thread's title. */
  readonly title = input.required<string>();
  /** The name of the space it is in, if known. */
  readonly space = input<string | null>(null);
  /** What the thread waits on or how it ended, if anything. */
  readonly mark = input<ThreadMark | null>(null);
  /** Emitted when the person asks to rename the thread. */
  readonly rename = output();

  protected readonly copy = COPY;
  protected readonly icons = { rename: Pencil, tools: PanelRight };
  private readonly panel = inject(ToolsPanelService);
  protected readonly panelOpen = this.panel.open;
  protected readonly narrow = this.panel.narrow;
  protected readonly shortcut = this.panel.ariaShortcut;

  protected startRename(): void {
    this.rename.emit();
  }

  protected togglePanel(): void {
    this.panel.toggle();
  }
}
