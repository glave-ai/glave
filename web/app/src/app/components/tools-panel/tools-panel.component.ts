import { ChangeDetectionStrategy, Component, computed, inject } from '@angular/core';
import {
  GlzIconButtonComponent,
  GlzIconComponent,
  GlzTabComponent,
  GlzTabsComponent,
} from '@glave/glaze';
import { X } from 'lucide';

import { changedFiles } from '../../lib/files';
import { ThreadService } from '../../services/thread.service';
import { ToolsPanelService } from '../../services/tools-panel.service';
import { TaskListComponent } from '../task-list/task-list.component';
import { ThreadFilesComponent } from '../thread-files/thread-files.component';

const COPY = {
  heading: 'Tools',
  close: 'Close the tools',
  tabs: 'Tools',
  tasks: 'Tasks',
  files: 'Files',
} as const;

/**
 * The thread's tools, beside it: the background tasks, and the files this thread changed.
 * Its tab is remembered with the panel.
 */
@Component({
  selector: 'app-tools-panel',
  imports: [
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
  protected readonly tab = this.panel.tabIndex;
  protected readonly filesLabel = computed(() => {
    const count = changedFiles(this.thread.log()).length;
    return count === 0 ? COPY.files : `${COPY.files} (${String(count)})`;
  });

  protected select(index: number): void {
    this.panel.showTab(index);
  }

  protected close(): void {
    this.panel.open.set(false);
  }
}
