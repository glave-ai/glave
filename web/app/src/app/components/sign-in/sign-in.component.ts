import { ChangeDetectionStrategy, Component, effect, inject } from '@angular/core';
import { Router } from '@angular/router';
import { GlzEmptyStateComponent } from '@glave/glaze';
import { KeyRound } from 'lucide';

import { SessionService } from '../../services/session.service';

const COPY = {
  heading: 'Sign in from your terminal',
  text: 'Run glave serve --http in your project, then open the link it prints. The link signs this browser in; the sign-in lasts while this tab is open.',
  command: 'glave serve --http 8080',
} as const;

/** Says how to sign in: with the link `glave serve --http` prints. */
@Component({
  selector: 'app-sign-in',
  imports: [GlzEmptyStateComponent],
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
