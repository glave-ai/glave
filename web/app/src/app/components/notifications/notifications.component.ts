import { ChangeDetectionStrategy, Component, LOCALE_ID, computed, inject } from '@angular/core';
import { GlzButtonComponent, GlzCardComponent, GlzCheckboxComponent } from '@glave/glaze';

import { PushService } from '../../services/push.service';

/**
 * In Settings, where the server pushes notifications: this browser's own, on or off (with why
 * it can't take them, where it can't), which kinds the person is told of, and every browser the
 * server pushes to, each to remove. Where the server pushes none, nothing, not even room.
 */
@Component({
  selector: 'app-notifications',
  imports: [GlzButtonComponent, GlzCardComponent, GlzCheckboxComponent],
  templateUrl: './notifications.component.html',
  styleUrl: './notifications.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class NotificationsComponent {
  private readonly push = inject(PushService);
  private readonly locale = inject(LOCALE_ID);
  protected readonly copy = {
    heading: $localize`:@@notifications.heading:Notifications`,
    lead: $localize`:@@notifications.lead:glave can tell you when it needs you, even with this tab closed.`,
    on: $localize`:@@notifications.on:This browser gets notifications.`,
    off: $localize`:@@notifications.off:This browser doesn’t get notifications.`,
    turnOn: $localize`:@@notifications.turnOn:Turn on`,
    turnOff: $localize`:@@notifications.turnOff:Turn off`,
    blocked: $localize`:@@notifications.blocked:Notifications are blocked for glave in this browser’s settings. Allow them there, then turn them on here.`,
    homeScreen: $localize`:@@notifications.homeScreen:On an iPhone or iPad, add glave to your Home Screen, then open it from there to get notifications.`,
    unsupported: $localize`:@@notifications.unsupported:This browser can’t show glave’s notifications.`,
    when: $localize`:@@notifications.when:Tell me when`,
    waiting: $localize`:@@notifications.waiting:glave needs me: an approval or a question`,
    finished: $localize`:@@notifications.finished:A task or schedule finishes or fails`,
    devices: $localize`:@@notifications.devices:Browsers that get them`,
    remove: $localize`:@@notifications.remove:Remove`,
  } as const;
  protected readonly shown = this.push.pushes;
  protected readonly homeScreen = this.push.homeScreenFirst;
  protected readonly unsupported = computed(() => !this.push.possible());
  protected readonly blocked = this.push.blocked;
  protected readonly busy = this.push.busy;
  protected readonly problem = this.push.problem;
  protected readonly on = this.push.on;
  protected readonly state = computed(() => (this.on() ? this.copy.on : this.copy.off));
  protected readonly toggle = computed(() => (this.on() ? this.copy.turnOff : this.copy.turnOn));
  protected readonly waiting = computed(() => this.kinds().waiting);
  protected readonly finished = computed(() => this.kinds().finished);
  /** The browsers the server pushes to, as the list shows them. */
  protected readonly devices = computed(() => {
    const day = new Intl.DateTimeFormat(this.locale, { dateStyle: 'medium' });
    return this.push.devices().map((device) => ({
      id: device.id,
      name: device.name,
      detail:
        device.current === true
          ? $localize`:@@notifications.thisBrowser:This browser · added ${day.format(new Date(device.added))}:DATE:`
          : $localize`:@@notifications.added:Added ${day.format(new Date(device.added))}:DATE:`,
      removeLabel: $localize`:@@notifications.removeOne:Remove ${device.name}:NAME:`,
    }));
  });
  protected readonly listed = computed(() => this.devices().length > 0);
  /** Which kinds the server pushes, as it said; every kind until it has. */
  private readonly kinds = computed(() => {
    const info = this.push.info();
    return info?.kind === 'offered' ? info : { waiting: true, finished: true };
  });

  constructor() {
    void this.push.load();
  }

  protected async switchHere(): Promise<void> {
    await (this.on() ? this.push.turnOff() : this.push.turnOn());
  }

  protected async setWaiting(waiting: boolean): Promise<void> {
    await this.push.choose(waiting, this.finished());
  }

  protected async setFinished(finished: boolean): Promise<void> {
    await this.push.choose(this.waiting(), finished);
  }

  protected async remove(device: string): Promise<void> {
    await this.push.remove(device);
  }
}
