import { ChangeDetectionStrategy, Component, computed, inject } from '@angular/core';
import type { ApprovalMode } from '@glave/client';
import { GlzIconComponent, type GlzIconData } from '@glave/glaze';
import { Hand, ShieldCheck } from 'lucide';

import { ServerService } from '../../services/server.service';

const COPY = {
  prefix: 'Approvals:',
  serverWide:
    'Set for this server with --approval or approval in config.toml; choosing it per thread comes later.',
} as const;

/** How each mode is shown. */
const MODES: Readonly<
  Record<
    ApprovalMode,
    { readonly text: string; readonly about: string; readonly icon: GlzIconData }
  >
> = {
  auto: {
    text: 'Auto',
    about: 'glave acts for you and asks only before risky actions.',
    icon: ShieldCheck,
  },
  ask: {
    text: 'Ask',
    about: 'glave asks before every change and every command.',
    icon: Hand,
  },
};

/**
 * The approval mode the thread runs in, read-only in the composer: the session API says the
 * server's mode (`server/discover`) but cannot set one per thread yet.
 */
@Component({
  selector: 'app-approval-mode',
  imports: [GlzIconComponent],
  templateUrl: './approval-mode.component.html',
  styleUrl: './approval-mode.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class ApprovalModeComponent {
  protected readonly copy = COPY;
  private readonly server = inject(ServerService);
  protected readonly mode = computed(() => {
    const mode = this.server.approval();
    if (mode === null) {
      return null;
    }
    const shown = MODES[mode];
    return { ...shown, title: `${shown.about} ${COPY.serverWide}` };
  });

  constructor() {
    void this.server.load();
  }
}
