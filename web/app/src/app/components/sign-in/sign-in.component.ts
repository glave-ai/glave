import {
  ChangeDetectionStrategy,
  Component,
  computed,
  effect,
  inject,
  signal,
} from '@angular/core';
import { Router } from '@angular/router';
import { GlzButtonComponent, GlzEmptyStateComponent, GlzTopBarComponent } from '@glave/glaze';
import { KeyRound } from 'lucide';

import { PasskeysService } from '../../services/passkeys.service';
import { SessionService } from '../../services/session.service';

const COPY = {
  name: 'glave',
  heading: 'Open glave again to sign in',
  text: 'For your safety, glave signs in only the browser it opens itself, and only while this tab stays open. Open glave on this computer again and it will bring you back here, signed in.',
  passkeyHeading: 'Sign in to glave',
  passkeyText:
    'Sign in with the passkey you made for this glave: your device asks for your face, your fingerprint or its screen lock.',
  passkeyHint:
    'No passkey on this device yet? On the computer glave runs on, ask glave for a sign-in link, and open it here.',
  passkeyAction: 'Sign in with a passkey',
  setupHeading: 'Make a passkey for this device',
  setupText:
    'This link lets this device sign in to glave. Your device makes a passkey, and from now on your face, your fingerprint or its screen lock signs you in. The link works once.',
  setupAction: 'Make a passkey',
  waiting: 'Waiting for your device…',
} as const;

/** What the page offers: a passkey to make, one to sign in with, or opening glave again. */
type Offer = 'setup' | 'passkey' | 'link';

/**
 * The page for someone not signed in. Where glave is reached from elsewhere, a passkey signs
 * them in, and a setup link makes the first; on the computer it runs on, glave opens the
 * browser signed in, so the page says to open it again.
 */
@Component({
  selector: 'app-sign-in',
  imports: [GlzButtonComponent, GlzEmptyStateComponent, GlzTopBarComponent],
  templateUrl: './sign-in.component.html',
  styleUrl: './sign-in.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class SignInComponent {
  private readonly session = inject(SessionService);
  private readonly passkeys = inject(PasskeysService);
  protected readonly copy = COPY;
  protected readonly icon = KeyRound;
  protected readonly busy = signal(false);
  protected readonly problem = signal<string | null>(null);

  protected readonly offer = computed<Offer>(() => {
    if (!this.session.passkeys()) {
      return 'link';
    }
    return this.session.setupCode() === null ? 'passkey' : 'setup';
  });
  protected readonly setup = computed(() => this.offer() === 'setup');
  protected readonly withPasskey = computed(() => this.offer() !== 'link');
  protected readonly heading = computed(() => {
    const headings: Record<Offer, string> = {
      setup: COPY.setupHeading,
      passkey: COPY.passkeyHeading,
      link: COPY.heading,
    };
    return headings[this.offer()];
  });
  protected readonly action = computed(() => {
    if (this.busy()) {
      return COPY.waiting;
    }
    return this.offer() === 'setup' ? COPY.setupAction : COPY.passkeyAction;
  });

  constructor() {
    const router = inject(Router);
    effect(() => {
      if (this.session.signedIn()) {
        void router.navigateByUrl('/');
      }
    });
  }

  /** Makes a passkey, or signs in with one, as the page offers; says why when it cannot. */
  protected async go(): Promise<void> {
    this.busy.set(true);
    this.problem.set(null);
    const refused =
      this.offer() === 'setup' ? await this.passkeys.make() : await this.passkeys.signIn();
    this.busy.set(false);
    this.problem.set(refused);
  }
}
