import {
  ChangeDetectionStrategy,
  Component,
  computed,
  effect,
  inject,
  signal,
} from '@angular/core';
import { Router, RouterLink } from '@angular/router';
import { GlzButtonComponent, GlzIconComponent, GlzTopBarComponent } from '@glave/glaze';
import { KeyRound } from 'lucide';

import { PasskeysService } from '../../services/passkeys.service';
import { SessionService } from '../../services/session.service';

/** What the page offers: a passkey to make, one to sign in with, or opening glave again. */
type Offer = 'setup' | 'passkey' | 'link';

/**
 * The page for someone not signed in: glave's name and a link to what it is, then one column
 * saying what glave is and how to sign in. Where glave is reached from elsewhere, a passkey
 * signs them in, and a setup link makes the first. On the computer it runs on, glave opens the
 * browser signed in and gives a link for any other, so the page says to open it; once a
 * browser here made a passkey, it signs any browser here in, in the words of the computer's
 * own authenticator ("Sign in with Touch ID").
 */
@Component({
  selector: 'app-sign-in',
  imports: [RouterLink, GlzButtonComponent, GlzIconComponent, GlzTopBarComponent],
  templateUrl: './sign-in.component.html',
  styleUrl: './sign-in.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class SignInComponent {
  private readonly session = inject(SessionService);
  private readonly passkeys = inject(PasskeysService);
  protected readonly copy = {
    name: 'glave',
    about: $localize`:@@signIn.about:What is Glave?`,
    headline: $localize`:@@signIn.headline:Your AI, on your own computer in the cloud.`,
    lede: $localize`:@@signIn.lede:Glave is an AI assistant with a computer of its own. It does real work for you, with your files, your apps and the web, and asks before anything that matters.`,
    inviteOnly: $localize`:@@signIn.inviteOnly:Glave is invite-only for now. Got an invite? Open its link to start.`,
    heading: $localize`:@@signIn.heading:Open glave on this computer to sign this browser in`,
    text: $localize`:@@signIn.text:Opening glave signs your browser in, and gives you a link that signs in any other browser on this computer. Once signed in, a browser stays signed in.`,
    passkeyHeading: $localize`:@@signIn.passkeyHeading:Sign in to glave`,
    passkeyHint: $localize`:@@signIn.passkeyHint:No passkey on this device yet? On the computer glave runs on, ask glave for a sign-in link, and open it here.`,
    hereText: $localize`:@@signIn.hereText:Sign in with the passkey you made for glave on this computer.`,
    hereHint: $localize`:@@signIn.hereHint:No passkey in this browser? Open glave on this computer: it signs this browser in.`,
    passkeyAction: $localize`:@@signIn.passkeyAction:Sign in with a passkey`,
    setupHeading: $localize`:@@signIn.setupHeading:Make a passkey for this device`,
    setupText: $localize`:@@signIn.setupText:This link lets this device sign in to glave. Your device makes a passkey, and from now on your face, your fingerprint or its screen lock signs you in. The link works once.`,
    setupAction: $localize`:@@signIn.setupAction:Make a passkey`,
    waiting: $localize`:@@signIn.waiting:Waiting for your device…`,
    lost: $localize`:@@signIn.lost:Lost your passkey?`,
    recover: $localize`:@@signIn.recover:Email me a link to make a new one`,
    signUp: $localize`:@@signIn.signUp:Sign up`,
  } as const;
  /** Where "What is Glave?" leads: glave's own site. */
  protected readonly aboutLink = 'https://glave.dev';
  protected readonly signUpLink = '/sign-up';
  protected readonly recoverLink = '/recover';
  protected readonly key = KeyRound;
  protected readonly busy = signal(false);
  /** Whether anyone may sign up here, so the page offers it beside signing in. */
  protected readonly signsUp = this.session.signsUp;
  /** Whether "Lost your passkey?" is offered: passkeys sign in here, and the server sends links to get back in. */
  protected readonly recovers = computed(() => this.withPasskey() && this.session.recovers());
  protected readonly problem = signal<string | null>(null);

  protected readonly offer = computed<Offer>(() => {
    if (this.session.setupCode() !== null && this.session.passkeys()) {
      return 'setup';
    }
    return this.session.passkeySignIn() ? 'passkey' : 'link';
  });
  protected readonly withPasskey = computed(() => this.offer() !== 'link');
  /** Whether it signs in with a passkey made on this computer, rather than elsewhere. */
  private readonly here = computed(() => this.session.passkeysHere());
  /** Whether the page greets someone reaching glave from elsewhere, who may not know it yet. */
  private readonly greets = computed(() => this.offer() === 'passkey' && !this.here());
  protected readonly heading = computed(() => {
    const headings: Record<Offer, string> = {
      setup: this.copy.setupHeading,
      passkey: this.here() ? this.copy.passkeyHeading : this.copy.headline,
      link: this.copy.heading,
    };
    return headings[this.offer()];
  });
  /** The muted words under the heading: what glave is, or how this page signs in. */
  protected readonly lede = computed(() => {
    const ledes: Record<Offer, string> = {
      setup: this.copy.setupText,
      passkey: this.here() ? this.copy.hereText : this.copy.lede,
      link: this.copy.text,
    };
    return ledes[this.offer()];
  });
  /**
   * The small line at the column's foot, if any: how to start where people join by invite,
   * else where a passkey for this device comes from, unless the page offers signing up.
   */
  protected readonly note = computed(() => {
    if (this.session.inviteOnly()) {
      return this.copy.inviteOnly;
    }
    if (this.greets()) {
      return this.signsUp() ? null : this.copy.passkeyHint;
    }
    return this.offer() === 'passkey' ? this.copy.hereHint : null;
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
