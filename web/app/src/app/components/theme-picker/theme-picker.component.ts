import { ChangeDetectionStrategy, Component, computed, inject } from '@angular/core';
import { GlzIconButtonComponent, GlzIconComponent, type GlzIconData } from '@glave/glaze';
import { Moon, Sun, SunMoon } from 'lucide';

import { type ThemeChoice, ThemeService } from '../../services/theme.service';

const ICONS: Readonly<Record<ThemeChoice, GlzIconData>> = {
  system: SunMoon,
  light: Sun,
  dark: Moon,
};

const LABELS: Readonly<Record<ThemeChoice, string>> = {
  system: 'Theme: as the system. Switch to light',
  light: 'Theme: light. Switch to Evening',
  dark: 'Theme: Evening. Switch to the system theme',
};

/** A button that moves between the system theme, light and Evening (dark). */
@Component({
  selector: 'app-theme-picker',
  imports: [GlzIconButtonComponent, GlzIconComponent],
  templateUrl: './theme-picker.component.html',
  styleUrl: './theme-picker.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class ThemePickerComponent {
  private readonly theme = inject(ThemeService);

  protected readonly icon = computed(() => ICONS[this.theme.choice()]);
  protected readonly label = computed(() => LABELS[this.theme.choice()]);

  protected cycle(): void {
    this.theme.cycle();
  }
}
