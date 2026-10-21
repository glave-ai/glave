import { ChangeDetectionStrategy, Component, LOCALE_ID, computed, inject } from '@angular/core';
import {
  GlzIconButtonComponent,
  GlzIconComponent,
  GlzTabComponent,
  GlzTabsComponent,
} from '@glave/glaze';
import { X } from 'lucide';

import { CommandsService } from '../../services/commands.service';
import { ThreadService } from '../../services/thread.service';
import { ToolsPanelService } from '../../services/tools-panel.service';
import { CommandListComponent } from '../command-list/command-list.component';
import { TaskListComponent } from '../task-list/task-list.component';
import { ThreadFilesComponent } from '../thread-files/thread-files.component';

/** A tab's label, with how many it holds (written for `locale`) when it holds any. */
function counted(label: string, count: number, locale: string): string {
  const number = new Intl.NumberFormat(locale).format(count);
  return count === 0 ? label : $localize`:@@toolsPanel.counted:${label}:TAB: (${number}:COUNT:)`;
}

/**
 * The thread's tools, beside it: the background tasks, the changes this thread made to files,
 * and the commands it keeps running in the background. Its tab is remembered with the panel. As
 * a sheet on a narrow screen it has a close button; beside the page, its edge collapses it.
 */
@Component({
  selector: 'app-tools-panel',
  imports: [
    CommandListComponent,
    GlzIconButtonComponent,
    GlzIconComponent,
    GlzTabComponent,
    GlzTabsComponent,
    TaskListComponent,
    ThreadFilesComponent,
  ],
  templateUrl: './tools-panel.component.html',
  styleUrl: './tools-panel.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class ToolsPanelComponent {
  protected readonly copy = {
    heading: $localize`:@@toolsPanel.heading:Tools`,
    close: $localize`:@@toolsPanel.close:Close the tools`,
    tabs: $localize`:@@toolsPanel.tabs:Tools`,
    tasks: $localize`:@@toolsPanel.tasks:Tasks`,
    changes: $localize`:@@toolsPanel.changes:Changes`,
    commands: $localize`:@@toolsPanel.commands:Commands`,
  } as const;
  protected readonly closeIcon = X;
  private readonly panel = inject(ToolsPanelService);
  private readonly thread = inject(ThreadService);
  private readonly commands = inject(CommandsService);
  private readonly locale = inject(LOCALE_ID);
  protected readonly tab = this.panel.tabIndex;
  /** Whether the panel is a sheet over the page, which has a close button of its own. */
  protected readonly narrow = this.panel.narrow;
  protected readonly changesLabel = computed(() =>
    counted(this.copy.changes, this.thread.files().length, this.locale),
  );
  protected readonly commandsLabel = computed(() =>
    counted(this.copy.commands, this.commands.count(), this.locale),
  );

  protected select(index: number): void {
    this.panel.showTab(index);
  }

  protected close(): void {
    this.panel.open.set(false);
  }
}
