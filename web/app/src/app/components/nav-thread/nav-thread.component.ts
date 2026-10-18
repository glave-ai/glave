import { ChangeDetectionStrategy, Component, input, output } from '@angular/core';
import { RouterLink } from '@angular/router';
import { GlzBadgeComponent, GlzNavItemComponent } from '@glave/glaze';

import type { NavThread } from '../../lib/nav';

/**
 * A thread's line in the side nav: its title on one line, cut short if need be, and its mark at
 * the end, quiet (a dot for what needs the person, the word alone while it runs); under it,
 * indented on a thin line from it, the threads it started, each a line of its own.
 */
@Component({
  selector: 'app-nav-thread',
  imports: [RouterLink, GlzBadgeComponent, GlzNavItemComponent],
  templateUrl: './nav-thread.component.html',
  styleUrl: './nav-thread.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class NavThreadComponent {
  readonly thread = input.required<NavThread>();
  /** Emitted when the person follows the thread's link, or one under it, so a drawer can close. */
  readonly navigated = output();

  protected followed(): void {
    this.navigated.emit();
  }
}
