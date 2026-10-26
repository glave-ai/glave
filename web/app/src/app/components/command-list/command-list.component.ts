import { ChangeDetectionStrategy, Component, LOCALE_ID, computed, inject } from '@angular/core';
import type { CommandId } from '@glave/client';
import {
  GlzIconButtonComponent,
  GlzIconComponent,
  GlzTooltipDirective,
  formatDuration,
} from '@glave/glaze';
import { LoaderCircle, Square } from 'lucide';

import { runningFor } from '../../lib/when';
import { CommandsService } from '../../services/commands.service';

/**
 * The thread's commands running in the background, a row each: a turning ring, what it runs,
 * in one line of monospace (all of it on hover or focus), how long it has run, ticking, and
 * Stop, which shows it stopping until it goes; a refusal under its row, in the server's words.
 */
@Component({
  selector: 'app-command-list',
  imports: [GlzIconButtonComponent, GlzIconComponent, GlzTooltipDirective],
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
