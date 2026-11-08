import { ChangeDetectionStrategy, Component, LOCALE_ID, computed, inject } from '@angular/core';
import type { CommandId } from '@glave/client';
import { GlzIconButtonComponent, GlzIconComponent, formatDuration } from '@glave/glaze';
import { AppWindow, LoaderCircle, Square } from 'lucide';

import type { Preview } from '../../lib/previews';
import { runningFor } from '../../lib/when';
import { CommandsService } from '../../services/commands.service';
import { ThreadPanelService } from '../../services/thread-panel.service';

/**
 * The thread's commands running in the background, a line each: a turning ring, what it runs
 * in monospace, cut short, how long it has run, ticking, Open preview where it serves something
 * the person can open here, which shows it in the panel's Preview tab, and Stop, a quiet button
 * that shows it stopping until it goes. The line opens to the whole command and the server's
 * sentence about it; under it, where it listens and a refusal, each in the server's words.
 */
@Component({
  selector: 'app-command-list',
  imports: [GlzIconButtonComponent, GlzIconComponent],
  templateUrl: './command-list.component.html',
  styleUrl: './command-list.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class CommandListComponent {
  protected readonly icons = { running: LoaderCircle, preview: AppWindow, stop: Square } as const;
  protected readonly service = inject(CommandsService);
  private readonly panel = inject(ThreadPanelService);
  private readonly locale = inject(LOCALE_ID);

  protected readonly rows = computed(() => {
    const now = this.service.now();
    const stopping = this.service.stopping();
    const refusals = this.service.refusals();
    const previews = this.service.previews();
    return this.service.commands().map((command) => {
      const ran = runningFor(command.started_at, now);
      const busy = stopping.has(command.id);
      const what = command.command;
      return {
        id: command.id,
        preview: previews.find((preview) => preview.command === command.id) ?? null,
        previewLabel: $localize`:@@commandList.openPreview:Open the preview of ${what}:COMMAND:`,
        command: what,
        summary: command.summary,
        time: ran === null ? null : formatDuration(ran, this.locale),
        listening: command.listens ?? null,
        busy,
        ariaBusy: busy ? 'true' : null,
        actionLabel: busy
          ? $localize`:@@commandList.stoppingCommand:Stopping ${what}:COMMAND:…`
          : $localize`:@@commandList.stopCommand:Stop ${what}:COMMAND:`,
        refusal: refusals.get(command.id) ?? null,
      };
    });
  });

  protected stop(command: CommandId): void {
    void this.service.stop(command);
  }

  protected open(preview: Preview): void {
    this.panel.showPreview(preview);
  }
}
