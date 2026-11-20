import { LiveAnnouncer } from '@angular/cdk/a11y';
import {
  ChangeDetectionStrategy,
  Component,
  booleanAttribute,
  computed,
  inject,
  input,
  model,
  output,
  signal,
} from '@angular/core';
import type { ScheduleId, ScheduleInfo } from '@glave/client';
import {
  GlzButtonComponent,
  GlzConfirmDialogComponent,
  GlzIconComponent,
  GlzMenuButtonComponent,
  type GlzOption,
} from '@glave/glaze';
import { Pause, Play, Trash2 } from 'lucide';

import { SchedulesService } from '../../services/schedules.service';

/** What the ⋯ menu offers, by value. */
const TOGGLE = 'toggle';
const REMOVE = 'remove';

/**
 * What can be done with a schedule, wherever it shows (its chat's line, the Repeating list):
 * Pause, or Resume once paused, and Remove, a quiet button in the danger tone, which asks once
 * and keeps its chat with its past runs; as buttons, or under a ⋯ menu (`menu`, with any
 * `extras` after them) where a bar keeps to one row on a phone, each a finger's size there.
 * While a change is on its way the buttons wait; what came of it is announced, the schedule as the server answered emitted (`changed`,
 * `removed`), and what the server refused it with is put in `error`, in its words, for what
 * holds the actions to show.
 */
@Component({
  selector: 'app-schedule-actions',
  imports: [
    GlzButtonComponent,
    GlzConfirmDialogComponent,
    GlzIconComponent,
    GlzMenuButtonComponent,
  ],
  templateUrl: './schedule-actions.component.html',
  styleUrl: './schedule-actions.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
  host: { '[attr.aria-busy]': 'waiting()' },
})
export class ScheduleActionsComponent {
  readonly schedule = input.required<ScheduleInfo>();
  /** Whether the actions sit under a ⋯ menu, as in a phone's bar, rather than as buttons. */
  readonly menu = input(false, { transform: booleanAttribute });
  /** The test hook; its parts get `<testId>-pause`, `-remove`, `-menu` and `-confirm`. */
  readonly testId = input.required<string>();
  /** What the server refused the last change with, in its words, until the next is asked. */
  readonly error = model<string | null>(null);
  /** What else the ⋯ menu offers, after Pause and Remove, such as what a bar's button did. */
  readonly extras = input<readonly GlzOption[]>([]);
  /** Emitted with the value of one of `extras` when the person picks it. */
  readonly extra = output<string>();
  /** The schedule as the server answered a pause or a resume. */
  readonly changed = output<ScheduleInfo>();
  /** Emitted once the server has removed the schedule. */
  readonly removed = output();

  protected readonly copy = {
    pause: $localize`:@@scheduleActions.pause:Pause`,
    resume: $localize`:@@scheduleActions.resume:Resume`,
    remove: $localize`:@@scheduleActions.remove:Remove`,
    removeText: $localize`:@@scheduleActions.removeText:No more runs start. Its chat stays, with its past runs.`,
    paused: $localize`:@@scheduleActions.paused:Paused`,
    resumed: $localize`:@@scheduleActions.resumed:Resumed`,
    removed: $localize`:@@scheduleActions.removed:Removed`,
  } as const;
  protected readonly icons = { pause: Pause, resume: Play, remove: Trash2 };
  private readonly schedules = inject(SchedulesService);
  private readonly announcer = inject(LiveAnnouncer);
  protected readonly confirming = signal(false);
  private readonly busy = signal(false);
  /** While a change is on its way: the buttons wait, and say so. */
  protected readonly waiting = computed(() => (this.busy() ? 'true' : null));
  protected readonly paused = computed(() => this.schedule().paused);
  /** What removing it asks, in the server's words, naming it. */
  protected readonly stop = computed(() => this.schedule().stop);
  protected readonly ids = computed(() => {
    const id = this.testId();
    return {
      pause: `${id}-pause`,
      remove: `${id}-remove`,
      menu: `${id}-menu`,
      confirm: `${id}-confirm`,
    };
  });
  /** Each control's words, the schedule named for those of a screen reader. */
  protected readonly labels = computed(() => {
    const name = this.schedule().name_shown;
    const paused = this.paused();
    return {
      toggle: paused ? this.copy.resume : this.copy.pause,
      toggleNamed: paused
        ? $localize`:@@scheduleActions.resumeNamed:Resume ${name}:NAME:`
        : $localize`:@@scheduleActions.pauseNamed:Pause ${name}:NAME:`,
      removeNamed: $localize`:@@scheduleActions.removeNamed:Remove ${name}:NAME:`,
      menu: $localize`:@@scheduleActions.menu:Options for ${name}:NAME:`,
    };
  });
  protected readonly toggleIcon = computed(() =>
    this.paused() ? this.icons.resume : this.icons.pause,
  );
  protected readonly actions = computed<readonly GlzOption[]>(() => [
    { value: TOGGLE, label: this.labels().toggle },
    { value: REMOVE, label: this.copy.remove },
    ...this.extras(),
  ]);

  /** Pauses the schedule, or resumes it once paused. */
  protected toggle(): void {
    void this.flip();
  }

  protected ask(): void {
    if (!this.busy()) {
      this.confirming.set(true);
    }
  }

  /** Removes the schedule, when the person said yes. */
  protected decided(yes: boolean): void {
    if (yes) {
      void this.remove();
    }
  }

  protected act(value: string): void {
    if (value === TOGGLE) {
      this.toggle();
    } else if (value === REMOVE) {
      this.ask();
    } else {
      this.extra.emit(value);
    }
  }

  private async flip(): Promise<void> {
    const { schedule, paused } = this.schedule();
    const now = await this.change(schedule, () => this.schedules.pause(schedule, !paused));
    if (now !== null) {
      this.changed.emit(now);
      void this.announcer.announce(now.paused ? this.copy.paused : this.copy.resumed);
    }
  }

  private async remove(): Promise<void> {
    const schedule = this.schedule().schedule;
    const done = await this.change(schedule, async () => {
      await this.schedules.remove(schedule);
      return true;
    });
    if (done !== null) {
      this.removed.emit();
      void this.announcer.announce(this.copy.removed);
    }
  }

  /**
   * Asks for `change` to `schedule`, one at a time: what it answers, or `null` when it was refused,
   * waits for another, or answers once these actions show another schedule (a page that moved to
   * another chat meanwhile), which it then leaves alone, its answer and its refusal both dropped.
   */
  private async change<T>(schedule: ScheduleId, change: () => Promise<T>): Promise<T | null> {
    if (this.busy()) {
      return null;
    }
    this.busy.set(true);
    this.error.set(null);
    const still = (): boolean => this.schedule().schedule === schedule;
    try {
      const answer = await change();
      return still() ? answer : null;
    } catch (error: unknown) {
      if (still()) {
        this.error.set(error instanceof Error ? error.message : String(error));
      }
      return null;
    } finally {
      this.busy.set(false);
    }
  }
}
