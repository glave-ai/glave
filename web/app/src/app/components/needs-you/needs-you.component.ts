import { ChangeDetectionStrategy, Component, computed, inject, output } from '@angular/core';
import { RouterLink } from '@angular/router';
import {
  GlzBadgeComponent,
  GlzNavItemComponent,
  GlzPopoverComponent,
  GlzPopoverTriggerDirective,
} from '@glave/glaze';
import { Inbox } from 'lucide';

import { waitingThreads } from '../../lib/nav';
import { ProjectsService } from '../../services/projects.service';

const COPY = {
  label: 'Needs you',
  none: 'Nothing needs you right now.',
} as const;

/**
 * Needs you, at the foot of the side nav: how many threads wait for the person (an approval or
 * a question), and, opened, each of them with its project or Chats, to go straight to it.
 */
@Component({
  selector: 'app-needs-you',
  imports: [
    RouterLink,
    GlzBadgeComponent,
    GlzNavItemComponent,
    GlzPopoverComponent,
    GlzPopoverTriggerDirective,
  ],
  templateUrl: './needs-you.component.html',
  styleUrl: './needs-you.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class NeedsYouComponent {
  /** Emitted when the person goes to a thread, so a drawer can close. */
  readonly navigated = output();

  protected readonly copy = COPY;
  protected readonly icon = Inbox;
  private readonly projects = inject(ProjectsService);
  protected readonly threads = computed(() => waitingThreads(this.projects.projects()));
  protected readonly count = computed(() => this.threads().length);

  protected followed(): void {
    this.navigated.emit();
  }
}
