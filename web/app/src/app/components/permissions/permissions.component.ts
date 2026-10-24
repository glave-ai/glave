import { ChangeDetectionStrategy, Component, computed, inject, input, model } from '@angular/core';
import type { Permissions } from '@glave/client';
import {
  GlzIconComponent,
  type GlzIconData,
  type GlzMenuOption,
  GlzMenuSelectComponent,
} from '@glave/glaze';
import { Eye, ShieldCheck, ShieldOff } from 'lucide';

import { ServerService } from '../../services/server.service';

/**
 * What the agent may do in the composer's next turn, quiet in its status line: its icon and its
 * word, the shape and the word saying it, never a colour. The person's pick, else the thread's
 * own (`current`, from its summary), else the server's. A choice among the presets the server
 * lets a thread choose (`server/discover`'s `presets`), each with a line saying what it means;
 * where the server says Workspace asks as Read-only does (no sandbox there), its line is the
 * server's sentence. Read-only when it lets only one. `value` is the person's pick, `null` until
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
  /** The thread's own preset, or `null` for the server's. */
  readonly current = input<Permissions | null>(null);
  /** The person's pick, or `null` for none. */
  readonly value = model<Permissions | null>(null);

  protected readonly copy = {
    label: $localize`:@@permissions.label:Permissions`,
    prefix: $localize`:@@permissions.prefix:Permissions:`,
  } as const;
  /** How each preset is shown. */
  private readonly presets: Readonly<
    Record<
      Permissions,
      { readonly text: string; readonly about: string; readonly icon: GlzIconData }
    >
  > = {
    'read-only': {
      text: $localize`:@@permissions.readOnly:Read-only`,
      about: $localize`:@@permissions.readOnlyAbout:Looks around; asks before changing anything`,
      icon: Eye,
    },
    workspace: {
      text: $localize`:@@permissions.workspace:Workspace`,
      about: $localize`:@@permissions.workspaceAbout:Edits and runs commands here; asks before risky actions`,
      icon: ShieldCheck,
    },
    'full-access': {
      text: $localize`:@@permissions.fullAccess:Full access`,
      about: $localize`:@@permissions.fullAccessAbout:No sandbox; for work you fully trust`,
      icon: ShieldOff,
    },
  };
  private readonly server = inject(ServerService);
  /** The preset the next turn runs with. */
  private readonly shownPreset = computed(
    () => this.value() ?? this.current() ?? this.server.permissions(),
  );
  protected readonly shown = computed<string | null>(() => this.shownPreset());
  protected readonly choosable = computed(() => this.server.presets().length > 1);
  protected readonly options = computed<readonly GlzMenuOption[]>(() =>
    this.server.presets().map((preset) => ({
      value: preset,
      label: this.presets[preset].text,
      hint: this.about(preset),
      icon: this.presets[preset].icon,
    })),
  );
  /** The preset shown read-only, when there is no choice to make. */
  protected readonly fixed = computed(() => {
    const preset = this.shownPreset();
    if (preset === null || this.choosable()) {
      return null;
    }
    const shown = this.presets[preset];
    return {
      ...shown,
      title: $localize`:@@permissions.onlyOne:${this.about(preset)}:ABOUT:. The only choice this server allows.`,
    };
  });

  constructor() {
    void this.server.load();
  }

  protected pick(value: string | null): void {
    const preset = this.server.presets().find((each) => each === value);
    if (preset !== undefined) {
      this.value.set(preset);
    }
  }

  /** What `preset` means here: the server's sentence for Workspace where it asks as Read-only. */
  private about(preset: Permissions): string {
    const unsandboxed = this.server.unsandboxed();
    if (preset === 'workspace' && unsandboxed !== null) {
      return unsandboxed;
    }
    return this.presets[preset].about;
  }
}
