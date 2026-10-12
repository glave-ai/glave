import { ChangeDetectionStrategy, Component, computed, inject, input, model } from '@angular/core';
import type { ApprovalMode } from '@glave/client';
import {
  GlzIconComponent,
  type GlzIconData,
  type GlzMenuOption,
  GlzMenuSelectComponent,
} from '@glave/glaze';
import { Hand, ShieldCheck } from 'lucide';

import { ServerService } from '../../services/server.service';

const COPY = {
  label: 'Approvals',
  prefix: 'Approvals:',
  only: 'The only mode this server’s policy allows.',
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
    about: 'Acts for you; asks only before risky actions',
    icon: ShieldCheck,
  },
  ask: {
    text: 'Ask',
    about: 'Asks before every change and command',
    icon: Hand,
  },
};

/**
 * The approval mode of the composer's next turn: the person's pick, else the thread's own
 * (`current`, from its summary), else the server's. A choice among the modes the server lets a
 * thread choose (`server/discover`'s `approvals`); read-only when it lets only one. `value` is
 * the person's pick, `null` until they make one; it goes with the next `thread/start` or
 * `turn/start`.
 */
@Component({
  selector: 'app-approval-mode',
  imports: [GlzIconComponent, GlzMenuSelectComponent],
  templateUrl: './approval-mode.component.html',
  styleUrl: './approval-mode.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class ApprovalModeComponent {
  /** The thread's own mode, or `null` for the server's. */
  readonly current = input<ApprovalMode | null>(null);
  /** The person's pick, or `null` for none. */
  readonly value = model<ApprovalMode | null>(null);

  protected readonly copy = COPY;
  private readonly server = inject(ServerService);
  /** The mode the next turn runs in. */
  private readonly shownMode = computed(
    () => this.value() ?? this.current() ?? this.server.approval(),
  );
  protected readonly shown = computed<string | null>(() => this.shownMode());
  protected readonly choosable = computed(() => this.server.approvals().length > 1);
  protected readonly options = computed<readonly GlzMenuOption[]>(() =>
    this.server.approvals().map((mode) => ({
      value: mode,
      label: MODES[mode].text,
      hint: MODES[mode].about,
    })),
  );
  /** The mode shown read-only, when there is no choice to make. */
  protected readonly fixed = computed(() => {
    const mode = this.shownMode();
    if (mode === null || this.choosable()) {
      return null;
    }
    const shown = MODES[mode];
    return { ...shown, title: `${shown.about}. ${COPY.only}` };
  });

  constructor() {
    void this.server.load();
  }

  protected pick(value: string | null): void {
    const mode = this.server.approvals().find((each) => each === value);
    if (mode !== undefined) {
      this.value.set(mode);
    }
  }
}
