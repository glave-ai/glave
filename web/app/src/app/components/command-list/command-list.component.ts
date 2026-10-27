import { ChangeDetectionStrategy, Component, LOCALE_ID, computed, inject } from '@angular/core';
import type { CommandId } from '@glave/client';
import { GlzIconButtonComponent, GlzIconComponent, formatDuration } from '@glave/glaze';
import { LoaderCircle, Square } from 'lucide';

import { runningFor } from '../../lib/when';
import { CommandsService } from '../../services/commands.service';

/**
 * The thread's commands running in the background, a line each: a turning ring, what it runs
 * in monospace, cut short, how long it has run, ticking, and Stop, a quiet button that shows it
 * stopping until it goes. The line opens to the whole command and the server's sentence about
 * it; a refusal shows under it, in the server's words.
 */
@Component({
  selector: 'app-command-list',
  imports: [GlzIconButtonComponent, GlzIconComponent],
  templateUrl: './command-list.component.html',
  styleUrl: './command-list.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class CommandListComponent {
  protected readonly icons = { running: LoaderCircle, stop: Square } as const;
  protected readonly service = inject(CommandsService);
  private readonly locale = inject(LOCALE_ID);

  protected readonly rows = computed(() => {
    const now = this.service.now();
    const stopping = this.service.stopping();
    const refusals = this.service.refusals();
    return this.service.commands().map((command) => {
      const ran = runningFor(command.started_at, now);
      const busy = stopping.has(command.id);
      const what = command.command;
      return {
        id: command.id,
        command: what,
        summary: command.summary,
        time: ran === null ? null : formatDuration(ran, this.locale),
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
}
