import {
  ChangeDetectionStrategy,
  ChangeDetectorRef,
  Component,
  computed,
  inject,
  signal,
  viewChild,
} from '@angular/core';
import { ApiError, type AwakeInfo } from '@glave/client';
import { GlzCardComponent, GlzCheckboxComponent, GlzFormFieldComponent } from '@glave/glaze';

import { sentence, unreached } from '../../lib/text';
import { SessionService } from '../../services/session.service';

/**
 * In Settings, on the person's own computer: whether glave keeps it from going to sleep on its
 * own while a turn, task or schedule runs, as the server says, with a switch; or, where it
 * can't, the server's sentence saying why. On a server, which doesn't sleep, nothing, not even
 * room in the page.
 */
@Component({
  selector: 'app-awake',
  imports: [GlzCardComponent, GlzCheckboxComponent, GlzFormFieldComponent],
  templateUrl: './awake.component.html',
  styleUrl: './awake.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class AwakeComponent {
  private readonly session = inject(SessionService);
  private readonly changes = inject(ChangeDetectorRef);
  /** What the server last said; none until it has. */
  private readonly awake = signal<AwakeInfo | null>(null);
  protected readonly copy = {
    heading: $localize`:@@awake.heading:This computer`,
    label: $localize`:@@awake.label:While glave works`,
    keep: $localize`:@@awake.keep:Keep this computer awake`,
    hint: $localize`:@@awake.hint:The screen may still turn off, and closing the lid still puts it to sleep.`,
  } as const;
  protected readonly shown = computed(() => {
    const awake = this.awake();
    return awake !== null && awake.kind !== 'never';
  });
  protected readonly on = computed(() => {
    const awake = this.awake();
    return awake?.kind === 'chosen' && awake.on;
  });
  /** The switch, which shows the person's click at once, then what the server says. */
  private readonly box = viewChild(GlzCheckboxComponent);
  protected readonly cannot = computed(() => {
    const awake = this.awake();
    return awake?.kind === 'cannot' ? awake.reason : null;
  });
  /** Why the last change did not happen, until the next. */
  protected readonly problem = signal<string | null>(null);

  constructor() {
    void this.ask(() => this.session.call('awake/get', {}));
  }

  /** Turns keeping the computer awake on or off; the server's answer is what stays. */
  protected async set(on: boolean): Promise<void> {
    await this.ask(() => this.session.call('awake/set', { on }));
  }

  private async ask(asked: () => Promise<{ readonly awake: AwakeInfo }>): Promise<void> {
    try {
      this.awake.set((await asked()).awake);
      this.problem.set(null);
    } catch (error: unknown) {
      this.problem.set(error instanceof ApiError ? sentence(error.message) : unreached());
    }
    // The switch shows what the server said, whatever the click before it: the click's own
    // state reaches the page first, so that going back from it shows.
    this.changes.detectChanges();
    this.box()?.checked.set(this.on());
  }
}
