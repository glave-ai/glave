import { ChangeDetectionStrategy, Component, computed, inject, input, model } from '@angular/core';
import type { Permissions } from '@glave/client';
import { GlzIconComponent, type GlzMenuOption, GlzMenuSelectComponent } from '@glave/glaze';

import { modes } from '../../lib/modes';
import { ServerService } from '../../services/server.service';

/**
 * The composer's mode for its next turn, what the agent may do without asking, quiet in its
 * status line: its icon and its word, the shape and the word saying it, never a colour. The
 * person's pick, else the thread's own (`current`, from its summary), else the server's. A
 * choice among the modes the server lets a thread choose (`server/discover`'s `modes`), in the
 * server's order, each with a line saying what it means, Auto named as recommended; where the
 * server says Auto asks before everything (no sandbox there), its line is the server's sentence.
 * Shown without a menu when the server lets only one. `value` is the person's pick, `null` until
 * they make one; it goes with the next `thread/start` or `turn/start`.
 */
@Component({
  selector: 'app-permissions',
  imports: [GlzIconComponent, GlzMenuSelectComponent],
  templateUrl: './permissions.component.html',
  styleUrl: './permissions.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class PermissionsComponent {
  /** The thread's own mode, or `null` for the server's. */
  readonly current = input<Permissions | null>(null);
  /** The person's pick, or `null` for none. */
  readonly value = model<Permissions | null>(null);

  protected readonly copy = {
    label: $localize`:@@permissions.label:Mode`,
    prefix: $localize`:@@permissions.prefix:Mode:`,
  } as const;
  /** How each mode is shown. */
  private readonly modes = modes();
  private readonly server = inject(ServerService);
  /** The mode the next turn runs with. */
  private readonly shownMode = computed(
    () => this.value() ?? this.current() ?? this.server.permissions(),
  );
  protected readonly shown = computed<string | null>(() => this.shownMode());
  protected readonly choosable = computed(() => this.server.modes().length > 1);
  protected readonly options = computed<readonly GlzMenuOption[]>(() =>
    this.server.modes().map((mode) => {
      const shown = this.modes[mode];
      return {
        value: mode,
        label: shown.recommended
          ? $localize`:@@permissions.recommended:${shown.text}:MODE: · Recommended`
          : shown.text,
        short: shown.text,
        hint: this.about(mode),
        icon: shown.icon,
      };
    }),
  );
  /** The mode shown on its own, when there is no choice to make. */
  protected readonly fixed = computed(() => {
    const mode = this.shownMode();
    if (mode === null || this.choosable()) {
      return null;
    }
    const shown = this.modes[mode];
    return {
      ...shown,
      title: $localize`:@@permissions.onlyOne:${this.about(mode)}:ABOUT: The only choice this server allows.`,
    };
  });

  constructor() {
    void this.server.load();
  }

  protected pick(value: string | null): void {
    const mode = this.server.modes().find((each) => each === value);
    if (mode !== undefined) {
      this.value.set(mode);
    }
  }

  /** What `mode` means here: the server's sentence for Auto where it asks before everything. */
  private about(mode: Permissions): string {
    const unsandboxed = this.server.unsandboxed();
    if (mode === 'auto' && unsandboxed !== null) {
      return unsandboxed;
    }
    return this.modes[mode].about;
  }
}
