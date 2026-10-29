import {
  ChangeDetectionStrategy,
  ChangeDetectorRef,
  Component,
  computed,
  inject,
  signal,
  viewChild,
} from '@angular/core';
import { ApiError, type RememberingInfo } from '@glave/client';
import { GlzCardComponent, GlzCheckboxComponent, GlzFormFieldComponent } from '@glave/glaze';

import { sentence, unreached } from '../../lib/text';
import { onChange } from '../../services/changes.service';
import { SessionService } from '../../services/session.service';

/**
 * On the memory page: whether glave remembers for the person, as the server says, with a
 * switch. Where it is not theirs to choose (their organization turned memory off), the switch
 * shows off and can't be changed, with the server's sentence saying why.
 */
@Component({
  selector: 'app-memory-switch',
  imports: [GlzCardComponent, GlzCheckboxComponent, GlzFormFieldComponent],
  templateUrl: './memory-switch.component.html',
  styleUrl: './memory-switch.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class MemorySwitchComponent {
  private readonly session = inject(SessionService);
  private readonly changes = inject(ChangeDetectorRef);
  /** What the server last said; none until it has. */
  private readonly remembering = signal<RememberingInfo | null>(null);
  protected readonly copy = {
    heading: $localize`:@@memorySwitch.heading:Remembering`,
    label: $localize`:@@memorySwitch.label:Between your chats`,
    remember: $localize`:@@memorySwitch.remember:Let glave remember`,
    hint: $localize`:@@memorySwitch.hint:When it's off, your chats don't use or save memories. What glave saved stays here for you to edit or forget.`,
  } as const;
  protected readonly shown = computed(() => this.remembering() !== null);
  protected readonly on = computed(() => {
    const remembering = this.remembering();
    return remembering?.kind === 'chosen' && remembering.on;
  });
  /** Why it is not the person's to choose, in the server's words. */
  protected readonly never = computed(() => {
    const remembering = this.remembering();
    return remembering?.kind === 'never' ? sentence(remembering.reason) : null;
  });
  protected readonly locked = computed(() => this.never() !== null);
  protected readonly hint = computed(() => this.never() ?? this.copy.hint);
  /** The switch, which shows the person's click at once, then what the server says. */
  private readonly box = viewChild(GlzCheckboxComponent);
  /** Why the last change did not happen, until the next. */
  protected readonly problem = signal<string | null>(null);

  constructor() {
    const read = (): void => void this.ask(() => this.session.call('memory/get', {}));
    read();
    onChange('memory', read);
  }

  /** Turns memory on or off for the person; the server's answer is what stays. */
  protected async set(on: boolean): Promise<void> {
    await this.ask(() => this.session.call('memory/set', { on }));
  }

  private async ask(
    asked: () => Promise<{ readonly remembering: RememberingInfo }>,
  ): Promise<void> {
    try {
      this.remembering.set((await asked()).remembering);
      this.problem.set(null);
    } catch (error: unknown) {
      this.problem.set(error instanceof ApiError ? sentence(error.message) : unreached());
    }
    // The switch shows what the server said, whatever the click before it.
    this.changes.detectChanges();
    this.box()?.checked.set(this.on());
  }
}
