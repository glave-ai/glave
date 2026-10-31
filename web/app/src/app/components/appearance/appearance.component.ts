import { ChangeDetectionStrategy, Component, computed, inject } from '@angular/core';
import {
  GlzCardComponent,
  GlzCheckboxComponent,
  type GlzColorOption,
  GlzColorChoiceComponent,
  GlzSegmentedComponent,
} from '@glave/glaze';

import { AppearanceService, PLAIN } from '../../services/appearance.service';
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
  protected readonly accents: readonly GlzColorOption[] = [
    { value: PLAIN, label: $localize`:@@appearance.plain:No accent` },
    { value: '#0f766e', label: $localize`:@@appearance.teal:Teal`, color: '#0f766e' },
    { value: '#2f7d32', label: $localize`:@@appearance.green:Green`, color: '#2f7d32' },
    { value: '#d9580f', label: $localize`:@@appearance.orange:Orange`, color: '#d9580f' },
    { value: '#c62828', label: $localize`:@@appearance.red:Red`, color: '#c62828' },
    { value: '#6d3bd1', label: $localize`:@@appearance.purple:Purple`, color: '#6d3bd1' },
    { value: '#f2c200', label: $localize`:@@appearance.yellow:Yellow`, color: '#f2c200' },
  ];
  private readonly themeService = inject(ThemeService);
  private readonly service = inject(AppearanceService);

  protected readonly theme = this.themeService.choice;
  protected readonly value = this.service.value;
  protected readonly adjust = this.service.adjust;
  protected readonly problem = this.service.problem;
  protected readonly line = computed(() =>
    this.value() === PLAIN
      ? $localize`:@@appearance.usingPlain:You’re using glave’s plain look.`
      : $localize`:@@appearance.usingOwn:You’re using your own color.`,
  );
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
    if (this.value() === PLAIN) {
      this.service.tune(adjust);
    } else {
      void this.service.choose(this.value(), adjust);
    }
  }
}
