import {
  ChangeDetectionStrategy,
  Component,
  DestroyRef,
  LOCALE_ID,
  computed,
  inject,
  input,
  signal,
} from '@angular/core';
import { GlzButtonComponent, GlzConfirmDialogComponent } from '@glave/glaze';

import { nextOf } from '../../lib/repeating';
import { TasksService } from '../../services/tasks.service';

/**
 * Under a schedule's chat's title, a quiet line saying when it repeats, in the server's words,
 * and when it runs next or that it is paused, with Pause or Resume and Remove, which asks once
 * and keeps the chat. In any other chat, nothing.
 */
@Component({
  selector: 'app-repeat-line',
  imports: [GlzButtonComponent, GlzConfirmDialogComponent],
  templateUrl: './repeat-line.component.html',
  styleUrl: './repeat-line.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class RepeatLineComponent {
  /** The chat it is under. */
  readonly thread = input.required<string>();

  protected readonly copy = {
    pause: $localize`:@@repeatLine.pause:Pause`,
    resume: $localize`:@@repeatLine.resume:Resume`,
    remove: $localize`:@@repeatLine.remove:Remove`,
    removeHeading: $localize`:@@repeatLine.removeHeading:Stop repeating this?`,
    removeText: $localize`:@@repeatLine.removeText:No more runs start. This chat stays, with its past runs.`,
  } as const;
  private readonly service = inject(TasksService);
  private readonly locale = inject(LOCALE_ID);
  protected readonly confirming = signal(false);
  /** The schedule whose chat this is, if it is one. */
  protected readonly schedule = computed(
    () => this.service.schedules().find((each) => each.thread === this.thread()) ?? null,
  );
  protected readonly line = computed(() => {
    const schedule = this.schedule();
    if (schedule === null) {
      return null;
    }
    const next = nextOf(schedule, this.locale);
    return next === null ? schedule.repeats : `${schedule.repeats} · ${next}`;
  });
  protected readonly paused = computed(() => this.schedule()?.paused === true);

  constructor() {
    const unwatch = this.service.watch();
    inject(DestroyRef).onDestroy(unwatch);
  }

  /** Pauses the schedule, or resumes it once paused. */
  protected toggle(): void {
    const schedule = this.schedule();
    if (schedule !== null) {
      void this.service.pause(schedule.schedule, !schedule.paused);
    }
  }

  protected ask(): void {
    this.confirming.set(true);
  }

  /** Removes the schedule, when the person said yes. */
  protected decided(yes: boolean): void {
    const schedule = this.schedule();
    if (yes && schedule !== null) {
      void this.service.remove(schedule.schedule);
    }
  }
}
