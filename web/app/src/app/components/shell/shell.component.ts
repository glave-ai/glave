import { ChangeDetectionStrategy, Component } from '@angular/core';
import { RouterLink, RouterOutlet } from '@angular/router';
import { GlzTopBarComponent } from '@glave/glaze';

/** What the shell says. */
const COPY = {
  name: 'glave',
  skip: 'Skip to content',
} as const;

/** The frame around every page: the top bar, then the page. */
@Component({
  selector: 'app-shell',
  imports: [RouterOutlet, RouterLink, GlzTopBarComponent],
  templateUrl: './shell.component.html',
  styleUrl: './shell.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class ShellComponent {
  protected readonly copy = COPY;
  protected readonly mainLink = '#main';
}
