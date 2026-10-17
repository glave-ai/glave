import { ChangeDetectionStrategy, Component, computed, inject } from '@angular/core';
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

const COPY = {
  heading: 'Tools',
  close: 'Close the tools',
  tabs: 'Tools',
  tasks: 'Tasks',
  changes: 'Changes',
  commands: 'Commands',
} as const;

/** A tab's label, with how many it holds when it holds any. */
function counted(label: string, count: number): string {
  return count === 0 ? label : `${label} (${String(count)})`;
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
  protected readonly copy = COPY;
  protected readonly closeIcon = X;
  private readonly panel = inject(ToolsPanelService);
  private readonly thread = inject(ThreadService);
  private readonly commands = inject(CommandsService);
  protected readonly tab = this.panel.tabIndex;
  /** Whether the panel is a sheet over the page, which has a close button of its own. */
  protected readonly narrow = this.panel.narrow;
  protected readonly changesLabel = computed(() =>
    counted(COPY.changes, this.thread.files().length),
  );
  protected readonly commandsLabel = computed(() => counted(COPY.commands, this.commands.count()));

  protected select(index: number): void {
    this.panel.showTab(index);
  }

  protected close(): void {
    this.panel.open.set(false);
  }
}
