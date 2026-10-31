import { ChangeDetectionStrategy, Component, computed, inject } from '@angular/core';
import {
  GlzCardComponent,
  GlzCheckboxComponent,
  type GlzColorOption,
  GlzColorChoiceComponent,
  GlzSegmentedComponent,
} from '@glave/glaze';

import { personSwatches } from '../../lib/accents';
import { AppearanceService, ORGANIZATION, PLAIN, offered } from '../../services/appearance.service';
import { ThemeService, isChoice } from '../../services/theme.service';

/**
 * In Settings, for everyone: light, dark or as this device is, kept in this browser; and the
 * person's accent, kept by the server: none, a swatch, or any exact color, tuned so that text
 * stays easy to read unless they turn that off.
 */
@Component({
  selector: 'app-appearance',
  imports: [GlzCardComponent, GlzCheckboxComponent, GlzColorChoiceComponent, GlzSegmentedComponent],
  templateUrl: './appearance.component.html',
  styleUrl: './appearance.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class AppearanceComponent {
  protected readonly copy = {
    appearance: $localize`:@@appearance.heading:Appearance`,
    theme: $localize`:@@appearance.theme:Light or dark`,
    accent: $localize`:@@appearance.accent:Accent color`,
    any: $localize`:@@appearance.any:Pick your own color`,
    adjust: $localize`:@@appearance.adjust:Adjust for readability in light and dark`,
  } as const;
  protected readonly themes = [
    { value: 'system', label: $localize`:@@appearance.system:Match this device` },
    { value: 'light', label: $localize`:@@appearance.light:Light` },
    { value: 'dark', label: $localize`:@@appearance.dark:Dark` },
  ] as const;
  private readonly themeService = inject(ThemeService);
  private readonly service = inject(AppearanceService);

  protected readonly theme = this.themeService.choice;
  protected readonly value = this.service.value;
  protected readonly adjust = this.service.adjust;
  protected readonly problem = this.service.problem;
  private readonly organization = this.service.organization;
  /** Everyone uses the organization's accent, so the person chooses none. */
  protected readonly locked = computed(() => this.organization()?.locked ?? false);
  /** What the organization is called, as a person reads it in a sentence. */
  private readonly theirs = computed(
    () => this.organization()?.name ?? $localize`:@@appearance.yourOrganization:your organization`,
  );
  protected readonly accents = computed<readonly GlzColorOption[]>(() => {
    const name = this.theirs();
    return [
      ...(offered(this.organization())
        ? [{ value: ORGANIZATION, label: $localize`:@@appearance.theirs:${name}:NAME:’s color` }]
        : []),
      { value: PLAIN, label: $localize`:@@appearance.plain:No accent` },
      ...personSwatches(),
    ];
  });
  protected readonly line = computed(() => {
    const name = this.theirs();
    if (this.locked()) {
      return $localize`:@@appearance.usingLocked:Everyone in ${name}:NAME: uses the same color.`;
    }
    const value = this.value();
    if (value === ORGANIZATION) {
      return $localize`:@@appearance.usingTheirs:You’re using ${name}:NAME:’s color.`;
    }
    if (value === PLAIN) {
      return $localize`:@@appearance.usingPlain:You’re using glave’s plain look.`;
    }
    return offered(this.organization())
      ? $localize`:@@appearance.usingOwnBeside:You’re using your own color. ${name}:NAME:’s still applies to everyone else.`
      : $localize`:@@appearance.usingOwn:You’re using your own color.`;
  });
  protected readonly adjustLine = computed(() =>
    this.adjust()
      ? $localize`:@@appearance.adjusted:glave tunes your color a little so text stays easy to read.`
      : $localize`:@@appearance.exact:Your exact color, everywhere. Some text may be hard to read.`,
  );

  protected pickTheme(choice: string | null): void {
    if (isChoice(choice)) {
      this.themeService.set(choice);
    }
  }

  protected pickAccent(value: string | null): void {
    if (value !== null) {
      void this.service.choose(value, this.adjust());
    }
  }

  protected pickAdjust(adjust: boolean): void {
    const value = this.value();
    if (value === PLAIN || value === ORGANIZATION) {
      this.service.tune(adjust);
    } else {
      void this.service.choose(value, adjust);
    }
  }
}
