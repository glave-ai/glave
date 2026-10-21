import { ChangeDetectionStrategy, Component, input, output } from '@angular/core';
import { RouterLink } from '@angular/router';
import { GlzIconComponent, GlzNavItemComponent } from '@glave/glaze';
import { LoaderCircle } from 'lucide';

import { type NavThread, needsYou } from '../../lib/nav';

/**
 * A thread's line in the side nav: its title on one line, cut short if need be, and at its end
 * what the server says of it, quiet and in the ink of the text, no colour: a ring that turns
 * while it runs (still under reduced motion), a small mark while it waits for the person, else
 * how long ago it was last active. Screen readers hear "running", "needs you" or the time in
 * words. Under it, indented on a thin line from it, the threads it started, each a line of its
 * own.
 */
@Component({
  selector: 'app-nav-thread',
  imports: [RouterLink, GlzIconComponent, GlzNavItemComponent],
  templateUrl: './nav-thread.component.html',
  styleUrl: './nav-thread.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class NavThreadComponent {
  readonly thread = input.required<NavThread>();
  /** Emitted when the person follows the thread's link, or one under it, so a drawer can close. */
  readonly navigated = output();

  /** What says a thread runs: a ring that turns, and what screen readers hear. */
  protected readonly running = {
    icon: LoaderCircle,
    label: $localize`:@@navThread.running:running`,
  } as const;
  protected readonly needsYou = needsYou();

  protected followed(): void {
    this.navigated.emit();
  }
}
