import {
  ChangeDetectionStrategy,
  Component,
  LOCALE_ID,
  computed,
  inject,
  input,
  output,
} from '@angular/core';
import { GlzBadgeComponent, GlzNavListComponent } from '@glave/glaze';

import { statusSections, threadsWords } from '../../lib/nav';
import { ProjectsService } from '../../services/projects.service';
import { NavThreadComponent } from '../nav-thread/nav-thread.component';

/**
 * The person's threads in the side nav by what the server says each is doing: Needs you, with
 * how many, then Working, then Idle, each newest first, each thread once with its project, or
 * Chats, under its title. A section with nothing in it does not show. A search keeps the
 * threads it finds by title or by day, and says when it finds none.
 */
@Component({
  selector: 'app-status-nav',
  imports: [GlzBadgeComponent, GlzNavListComponent, NavThreadComponent],
  templateUrl: './status-nav.component.html',
  styleUrl: './status-nav.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class StatusNavComponent {
  /** The thread shown now, to mark it current. */
  readonly current = input<string | null>(null);
  /** What to search the thread titles for. */
  readonly query = input('');
  /** Emitted when the person follows a link, so a drawer can close. */
  readonly navigated = output();

  protected readonly copy = threadsWords();
  private readonly service = inject(ProjectsService);
  /** The reader's locale, which the threads' times and the count are written in. */
  private readonly locale = inject(LOCALE_ID);

  protected readonly sections = computed(() => {
    const { waiting, running, idle } = statusSections(this.service.projects(), {
      query: this.query(),
      current: this.current(),
      now: this.service.readAt(),
      reader: { locale: this.locale },
    });
    // How many wait, shown alone and read to screen readers in words.
    const count = new Intl.NumberFormat(this.locale).format(waiting.length);
    return [
      {
        testId: 'nav-needs-you',
        heading: $localize`:@@statusNav.needsYou:Needs you`,
        count: { count, words: $localize`:@@statusNav.waiting:${count}:COUNT: waiting` },
        threads: waiting,
      },
      {
        testId: 'nav-working',
        heading: $localize`:@@statusNav.working:Working`,
        count: null,
        threads: running,
      },
      {
        testId: 'nav-idle',
        heading: $localize`:@@statusNav.idle:Idle`,
        count: null,
        threads: idle,
      },
    ].filter(({ threads }) => threads.length > 0);
  });
  protected readonly noMatch = computed(
    () => this.query().trim() !== '' && this.sections().length === 0,
  );

  protected followed(): void {
    this.navigated.emit();
  }
}
