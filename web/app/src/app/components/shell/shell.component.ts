import { ChangeDetectionStrategy, Component } from '@angular/core';
import { RouterLink, RouterOutlet } from '@angular/router';
import { GlzTopBarComponent } from '@glave/glaze';

import { ThemePickerComponent } from '../theme-picker/theme-picker.component';

/** What the shell says. */
const COPY = {
  name: 'glave',
  skip: 'Skip to content',
} as const;

/** The frame around every page: the top bar, then the page. */
@Component({
  selector: 'app-shell',
  imports: [RouterOutlet, RouterLink, GlzTopBarComponent, ThemePickerComponent],
  templateUrl: './shell.component.html',
  styleUrl: './shell.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class ShellComponent {
  protected readonly copy = COPY;
  protected readonly mainLink = '#main';
}
