import {
  ChangeDetectionStrategy,
  Component,
  computed,
  effect,
  inject,
  signal,
} from '@angular/core';
import { Router, RouterLink } from '@angular/router';
import { GlzButtonComponent, GlzEmptyStateComponent, GlzTopBarComponent } from '@glave/glaze';
import { KeyRound } from 'lucide';

import { PasskeysService } from '../../services/passkeys.service';
import { SessionService } from '../../services/session.service';

/** What the page offers: a passkey to make, one to sign in with, or opening glave again. */
type Offer = 'setup' | 'passkey' | 'link';

/**
 * The page for someone not signed in. Where glave is reached from elsewhere, a passkey signs
 * them in, and a setup link makes the first. On the computer it runs on, glave opens the
 * browser signed in and gives a link for any other, so the page says to open it; once a
 * browser here made a passkey, it signs any browser here in, in the words of the computer's
 * own authenticator ("Sign in with Touch ID").
 */
@Component({
  selector: 'app-sign-in',
  imports: [RouterLink, GlzButtonComponent, GlzEmptyStateComponent, GlzTopBarComponent],
  templateUrl: './sign-in.component.html',
  styleUrl: './sign-in.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class SignInComponent {
  private readonly session = inject(SessionService);
  private readonly passkeys = inject(PasskeysService);
  protected readonly copy = {
    name: 'glave',
    heading: $localize`:@@signIn.heading:Open glave on this computer to sign this browser in`,
    text: $localize`:@@signIn.text:Opening glave signs your browser in, and gives you a link that signs in any other browser on this computer. Once signed in, a browser stays signed in.`,
    passkeyHeading: $localize`:@@signIn.passkeyHeading:Sign in to glave`,
    passkeyText: $localize`:@@signIn.passkeyText:Sign in with the passkey you made for this glave: your device asks for your face, your fingerprint or its screen lock.`,
    passkeyHint: $localize`:@@signIn.passkeyHint:No passkey on this device yet? On the computer glave runs on, ask glave for a sign-in link, and open it here.`,
    hereText: $localize`:@@signIn.hereText:Sign in with the passkey you made for glave on this computer.`,
    hereHint: $localize`:@@signIn.hereHint:No passkey in this browser? Open glave on this computer: it signs this browser in.`,
    passkeyAction: $localize`:@@signIn.passkeyAction:Sign in with a passkey`,
    setupHeading: $localize`:@@signIn.setupHeading:Make a passkey for this device`,
    setupText: $localize`:@@signIn.setupText:This link lets this device sign in to glave. Your device makes a passkey, and from now on your face, your fingerprint or its screen lock signs you in. The link works once.`,
    setupAction: $localize`:@@signIn.setupAction:Make a passkey`,
    waiting: $localize`:@@signIn.waiting:Waiting for your device…`,
    newHere: $localize`:@@signIn.newHere:New to glave?`,
    signUp: $localize`:@@signIn.signUp:Sign up`,
  } as const;
  protected readonly signUpLink = '/sign-up';
  protected readonly icon = KeyRound;
  protected readonly busy = signal(false);
  /** Whether anyone may sign up here, so the page offers it beside signing in. */
  protected readonly signsUp = this.session.signsUp;
  protected readonly problem = signal<string | null>(null);

  protected readonly offer = computed<Offer>(() => {
    if (this.session.setupCode() !== null && this.session.passkeys()) {
      return 'setup';
    }
    return this.session.passkeySignIn() ? 'passkey' : 'link';
  });
  protected readonly setup = computed(() => this.offer() === 'setup');
  protected readonly withPasskey = computed(() => this.offer() !== 'link');
  /** Whether it signs in with a passkey made on this computer, rather than elsewhere. */
  private readonly here = computed(() => this.session.passkeysHere());
  protected readonly passkeyText = computed(() =>
    this.here() ? this.copy.hereText : this.copy.passkeyText,
  );
  protected readonly passkeyHint = computed(() =>
    this.here() ? this.copy.hereHint : this.copy.passkeyHint,
  );
  protected readonly heading = computed(() => {
    const headings: Record<Offer, string> = {
      setup: this.copy.setupHeading,
      passkey: this.copy.passkeyHeading,
      link: this.copy.heading,
    };
    return headings[this.offer()];
  });
  protected readonly action = computed(() => {
    if (this.busy()) {
      return this.copy.waiting;
    }
    if (this.offer() === 'setup') {
      return this.copy.setupAction;
    }
    return this.here() ? this.signInHere() : this.copy.passkeyAction;
  });

  private readonly router = inject(Router);
  /** Signed in, by this page or another tab: on to the workspace. */
  protected readonly leaving = effect(() => {
    if (this.session.signedIn()) {
      void this.router.navigateByUrl('/');
    }
  });

  /** Signing in with this computer's own authenticator, by the name its system gives it. */
  private signInHere(): string {
    switch (this.passkeys.platform) {
      case 'touch-id':
        return $localize`:@@signIn.touchId:Sign in with Touch ID`;
      case 'windows-hello':
        return $localize`:@@signIn.windowsHello:Sign in with Windows Hello`;
      default:
        return this.copy.passkeyAction;
    }
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
