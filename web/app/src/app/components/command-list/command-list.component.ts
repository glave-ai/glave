import { ChangeDetectionStrategy, Component, LOCALE_ID, computed, inject } from '@angular/core';
import type { CommandId } from '@glave/client';
import {
  GlzButtonComponent,
  GlzCardComponent,
  GlzEmptyStateComponent,
  GlzTooltipDirective,
  formatDuration,
} from '@glave/glaze';
import { SquareTerminal } from 'lucide';

import { runningFor } from '../../lib/when';
import { CommandsService } from '../../services/commands.service';

/**
 * The thread's commands running in the background: what each runs, in one line of monospace
 * (all of it on hover or focus), the server's sentence about it, how long it has run, ticking,
 * and Stop, which shows it stopping until it goes.
 */
@Component({
  selector: 'app-command-list',
  imports: [GlzButtonComponent, GlzCardComponent, GlzEmptyStateComponent, GlzTooltipDirective],
  templateUrl: './command-list.component.html',
  styleUrl: './command-list.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class CommandListComponent {
  protected readonly copy = {
    stop: $localize`:@@commandList.stop:Stop`,
    stopping: $localize`:@@commandList.stopping:Stopping…`,
    emptyHeading: $localize`:@@commandList.emptyHeading:Nothing running in the background`,
    emptyText: $localize`:@@commandList.emptyText:When the agent keeps a command running, such as a dev server, it shows here.`,
  } as const;
  protected readonly icon = SquareTerminal;
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
      const time = ran === null ? null : formatDuration(ran, this.locale);
      return {
        id: command.id,
        command: what,
        summary: command.summary,
        time:
          time === null ? null : $localize`:@@commandList.runningFor:Running for ${time}:DURATION:`,
        busy,
        ariaBusy: busy ? 'true' : null,
        action: busy ? this.copy.stopping : this.copy.stop,
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
