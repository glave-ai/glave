import { ChangeDetectionStrategy, Component, computed, inject, signal } from '@angular/core';
import { GlzButtonComponent } from '@glave/glaze';

import { PushService } from '../../services/push.service';

/**
 * The quiet offer at the top of a thread (owner decision, 2026-11-07): a line saying the person
 * can be told when glave needs them, even with the tab closed, with Turn on and Not now. It is
 * drawn only where the server pushes notifications, this browser can show them, and the person
 * has not decided here yet, and nothing at all otherwise. It floats over the conversation, so
 * neither its coming nor its going moves the thread. Turn on asks the browser's permission, in
 * its click alone; Not now is remembered in this browser.
 */
@Component({
  selector: 'app-push-banner',
  imports: [GlzButtonComponent],
  templateUrl: './push-banner.component.html',
  styleUrl: './push-banner.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class PushBannerComponent {
  private readonly push = inject(PushService);
  protected readonly copy = {
    offer: $localize`:@@pushBanner.offer:Get a notification when glave needs you, even with this tab closed.`,
    turnOn: $localize`:@@pushBanner.turnOn:Turn on`,
    notNow: $localize`:@@pushBanner.notNow:Not now`,
  } as const;
  /** Whether Turn on was clicked here, so the banner stays to say what came of it. */
  private readonly turning = signal(false);
  protected readonly shown = computed(
    () => this.push.offered() || (this.turning() && this.push.problem() !== null),
  );
  protected readonly busy = this.push.busy;
  protected readonly problem = this.push.problem;

  protected turnOn(): void {
    this.turning.set(true);
    // Called in the click, so the browser may ask the person.
    void this.push.turnOn();
  }

  protected notNow(): void {
    this.turning.set(false);
    this.push.notNow();
  }
}
