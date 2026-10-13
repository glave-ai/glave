import { ChangeDetectionStrategy, Component, effect, inject } from '@angular/core';
import { Router } from '@angular/router';
import { GlzEmptyStateComponent, GlzTopBarComponent } from '@glave/glaze';
import { KeyRound } from 'lucide';

import { SessionService } from '../../services/session.service';

const COPY = {
  name: 'glave',
  heading: 'Open glave again to sign in',
  text: 'For your safety, glave signs in only the browser it opens itself, and only while this tab stays open. Open glave on this computer again and it will bring you back here, signed in.',
} as const;

/** The page for someone not signed in: glave opens the browser signed in, so open it again. */
@Component({
  selector: 'app-sign-in',
  imports: [GlzEmptyStateComponent, GlzTopBarComponent],
  templateUrl: './sign-in.component.html',
  styleUrl: './sign-in.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class SignInComponent {
  protected readonly copy = COPY;
  protected readonly icon = KeyRound;

  constructor() {
    const session = inject(SessionService);
    const router = inject(Router);
    effect(() => {
      if (session.signedIn()) {
        void router.navigateByUrl('/');
      }
    });
  }
}
