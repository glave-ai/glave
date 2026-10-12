import { ChangeDetectionStrategy, Component, inject } from '@angular/core';
import { GlzAvatarComponent, GlzIconButtonComponent, GlzIconComponent } from '@glave/glaze';
import { LogOut } from 'lucide';

import { SessionService } from '../../services/session.service';

const COPY = {
  you: 'You',
  signedIn: 'Signed in',
  signOut: 'Sign out of this tab',
} as const;

/**
 * The person signed in, at the foot of the side nav, with sign-out. The
 * session API does not say who someone is yet, so it says "You".
 */
@Component({
  selector: 'app-person',
  imports: [GlzAvatarComponent, GlzIconButtonComponent, GlzIconComponent],
  templateUrl: './person.component.html',
  styleUrl: './person.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class PersonComponent {
  protected readonly copy = COPY;
  protected readonly signOutIcon = LogOut;
  private readonly session = inject(SessionService);

  protected signOut(): void {
    this.session.signOut();
  }
}
