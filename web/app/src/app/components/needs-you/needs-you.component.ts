import { ChangeDetectionStrategy, Component, computed, input, output } from '@angular/core';
import { RouterLink } from '@angular/router';
import { GlzNavItemComponent, GlzNavListComponent } from '@glave/glaze';

import type { WaitingThread } from '../../lib/nav';

/**
 * Needs you, pinned at the top of the side nav while something waits for the person (an
 * approval, a question, a project's trust): each thread that waits, newest first, with its
 * project or Chats at its end (none with only Chats), to go straight to it. With none, nothing:
 * not even room in its parent's layout.
 */
@Component({
  selector: 'app-needs-you',
  imports: [RouterLink, GlzNavItemComponent, GlzNavListComponent],
  templateUrl: './needs-you.component.html',
  styleUrl: './needs-you.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class NeedsYouComponent {
  /** The threads that wait for the person. */
  readonly threads = input.required<readonly WaitingThread[]>();
  /** Emitted when the person goes to a thread, so a drawer can close. */
  readonly navigated = output();

  protected readonly copy = { label: $localize`:@@needsYou.label:Needs you` } as const;
  protected readonly shown = computed(() => this.threads().length > 0);

  protected followed(): void {
    this.navigated.emit();
  }
}
