import { ChangeDetectionStrategy, Component, inject, output } from '@angular/core';
import { RouterLink } from '@angular/router';
import {
  GlzAvatarComponent,
  GlzNavItemComponent,
  GlzPopoverComponent,
  GlzPopoverTriggerDirective,
} from '@glave/glaze';
import LogOut from 'lucide/dist/esm/icons/log-out.mjs';
import Settings from 'lucide/dist/esm/icons/settings.mjs';

import { SessionService } from '../../services/session.service';

/**
 * The person signed in, at the foot of the side nav: opened, their Settings and Sign out. The
 * session API does not say who someone is yet, so it says "You".
 */
@Component({
  selector: 'app-person',
  imports: [
    RouterLink,
    GlzAvatarComponent,
    GlzNavItemComponent,
    GlzPopoverComponent,
    GlzPopoverTriggerDirective,
  ],
  templateUrl: './person.component.html',
  styleUrl: './person.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class PersonComponent {
  /** Emitted when the person goes to a page, so a drawer can close. */
  readonly navigated = output();

  protected readonly copy = {
    you: $localize`:@@person.you:You`,
    settings: $localize`:@@person.settings:Settings`,
    signOut: $localize`:@@person.signOut:Sign out`,
  } as const;
  protected readonly icons = { settings: Settings, signOut: LogOut };
  protected readonly settingsLink = '/settings';
  private readonly session = inject(SessionService);

  protected followed(): void {
    this.navigated.emit();
  }

  protected signOut(): void {
    void this.session.signOut();
  }
}
