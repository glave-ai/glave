import { ChangeDetectionStrategy, Component, computed, inject } from '@angular/core';
import type { CommandId } from '@glave/client';
import {
  GlzButtonComponent,
  GlzCardComponent,
  GlzEmptyStateComponent,
  GlzTooltipDirective,
  formatDuration,
} from '@glave/glaze';
import { SquareTerminal } from 'lucide';

import { runningFor } from '../../lib/commands';
import { CommandsService } from '../../services/commands.service';

const COPY = {
  stop: 'Stop',
  stopping: 'Stopping…',
  runningFor: 'Running for',
  emptyHeading: 'Nothing running in the background',
  emptyText: 'When the agent keeps a command running, such as a dev server, it shows here.',
} as const;

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
  protected readonly copy = COPY;
  protected readonly icon = SquareTerminal;
  protected readonly service = inject(CommandsService);

  protected readonly rows = computed(() => {
    const now = this.service.now();
    const stopping = this.service.stopping();
    const refusals = this.service.refusals();
    return this.service.commands().map((command) => {
      const ran = runningFor(command.started_at, now);
      const busy = stopping.has(command.id);
      const action = busy ? COPY.stopping : COPY.stop;
      return {
        id: command.id,
        command: command.command,
        summary: command.summary,
        time: ran === null ? null : `${COPY.runningFor} ${formatDuration(ran)}`,
        busy,
        ariaBusy: busy ? 'true' : null,
        action,
        actionLabel: `${action} ${command.command}`,
        refusal: refusals.get(command.id) ?? null,
      };
    });
  });

  protected stop(command: CommandId): void {
    void this.service.stop(command);
  }
}
