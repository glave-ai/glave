import { ChangeDetectionStrategy, Component, computed, inject, signal } from '@angular/core';
import { ActivatedRoute, Router, RouterLink } from '@angular/router';
import { ApiError } from '@glave/client';
import {
  GlzButtonComponent,
  GlzEmptyStateComponent,
  GlzFormFieldComponent,
  GlzInputDirective,
  GlzTopBarComponent,
} from '@glave/glaze';
import KeyRound from 'lucide/dist/esm/icons/key-round.mjs';
import Mail from 'lucide/dist/esm/icons/mail.mjs';

import { unreached } from '../../lib/text';
import { PasskeysService } from '../../services/passkeys.service';
import { SessionService } from '../../services/session.service';

/** Where the person is in signing up. */
type Step = 'email' | 'sent' | 'passkey';

/** What an emailed link the page is for: signing up, or getting back in. */
type Emailed = 'signup' | 'recovery';

/** The page's words for signing up. */
function signUpCopy(): Copy {
  return {
    emailHeading: $localize`:@@signUp.emailHeading:Get started with glave`,
    emailText: $localize`:@@signUp.emailText:We'll send you a link. Your email is only for getting back in and for invites, never for ads.`,
    send: $localize`:@@signUp.send:Send me a link`,
    have: $localize`:@@signUp.have:Already have an account?`,
    passkeyHeading: $localize`:@@signUp.passkeyHeading:Make your passkey`,
    passkeyText: $localize`:@@signUp.passkeyText:This is how you'll sign in: your fingerprint, face or device code. No password.`,
    then: $localize`:@@signUp.then:Then you're in. Add a model and start a chat.`,
  };
}

/** The page's words for getting back in. */
function recoveryCopy(): Copy {
  return {
    emailHeading: $localize`:@@recover.emailHeading:Get back into glave`,
    emailText: $localize`:@@recover.emailText:Lost your passkey? Give your account's email, and we'll send a link that makes a new one on this device.`,
    send: $localize`:@@recover.send:Email me a link`,
    have: $localize`:@@recover.have:Found it?`,
    passkeyHeading: $localize`:@@recover.passkeyHeading:Make a new passkey`,
    passkeyText: $localize`:@@recover.passkeyText:This is how you'll sign in from now on: your fingerprint, face or device code.`,
    then: $localize`:@@recover.then:Then you're back in. Your other devices will be told.`,
  };
}

/** The words that differ between signing up and getting back in. */
interface Copy {
  readonly emailHeading: string;
  readonly emailText: string;
  readonly send: string;
  readonly have: string;
  readonly passkeyHeading: string;
  readonly passkeyText: string;
  readonly then: string;
}

/**
 * Signing up, where the server takes sign-ups, or getting back in, where it sends links for
 * that (`recover`, its route's `emailed`): the person's email, then a link the server sends
 * there, then, opened from it, a passkey: the first of their new account, or a new one for
 * theirs. Either signs them in. Nothing here speaks of organizations: a new account's is made
 * with it.
 */
@Component({
  selector: 'app-sign-up',
  imports: [
    RouterLink,
    GlzButtonComponent,
    GlzEmptyStateComponent,
    GlzFormFieldComponent,
    GlzInputDirective,
    GlzTopBarComponent,
  ],
  templateUrl: './sign-up.component.html',
  styleUrl: './sign-up.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class SignUpComponent {
  private readonly session = inject(SessionService);
  private readonly passkeys = inject(PasskeysService);
  private readonly router = inject(Router);
  private readonly emailed: Emailed =
    inject(ActivatedRoute).snapshot.data['emailed'] === 'recovery' ? 'recovery' : 'signup';
  protected readonly copy = {
    ...(this.emailed === 'recovery' ? recoveryCopy() : signUpCopy()),
    name: 'glave',
    email: $localize`:@@signUp.email:Email`,
    sending: $localize`:@@signUp.sending:Sending…`,
    signIn: $localize`:@@signUp.signIn:Sign in with your passkey`,
    sentHeading: $localize`:@@signUp.sentHeading:Check your email`,
    again: $localize`:@@signUp.again:Send it again`,
    make: $localize`:@@signUp.make:Make a passkey`,
    waiting: $localize`:@@signUp.waiting:Waiting for your device…`,
  } as const;
  /** The code of the emailed link the tab was opened with, of this page's kind. */
  private readonly code =
    this.emailed === 'recovery' ? this.session.recoverCode : this.session.signupCode;
  protected readonly icons = { mail: Mail, key: KeyRound } as const;
  protected readonly signInLink = '/sign-in';
  protected readonly email = signal('');
  private readonly sentTo = signal<string | null>(null);
  protected readonly busy = signal(false);
  protected readonly problem = signal<string | null>(null);

  protected readonly step = computed<Step>(() => {
    if (this.code() !== null) {
      return 'passkey';
    }
    return this.sentTo() === null ? 'email' : 'sent';
  });
  protected readonly sentText = computed(() => {
    const to = this.sentTo() ?? '';
    return this.emailed === 'recovery'
      ? $localize`:@@recover.sentText:If ${to}:EMAIL: has a glave account, we sent it a link. It works for 15 minutes and lets you make a new passkey. Your other devices will be told.`
      : $localize`:@@signUp.sentText:We sent a link to ${to}:EMAIL:. Open it on this device. It works for 15 minutes.`;
  });
  protected readonly sendLabel = computed(() => (this.busy() ? this.copy.sending : this.copy.send));
  protected readonly makeLabel = computed(() => (this.busy() ? this.copy.waiting : this.copy.make));

  protected typed(event: Event): void {
    if (event.target instanceof HTMLInputElement) {
      this.email.set(event.target.value);
    }
  }

  /** Asks the server to send the link to the email given. */
  protected async send(event?: Event): Promise<void> {
    event?.preventDefault();
    const email = this.email().trim();
    if (this.busy() || email === '') {
      return;
    }
    await this.doing(async () => {
      await this.session.call(this.emailed === 'recovery' ? 'recovery/start' : 'signup/start', {
        email,
      });
      this.sentTo.set(email);
      return null;
    });
  }

  /** Sends the link again, to the same email. */
  protected async again(): Promise<void> {
    await this.send();
  }

  /** Makes the passkey with the link the tab was opened with, then opens glave. */
  protected async make(): Promise<void> {
    const code = this.code();
    if (this.busy() || code === null) {
      return;
    }
    this.busy.set(true);
    const outcome = await this.passkeys.withEmailedLink(code, this.emailed);
    this.busy.set(false);
    this.problem.set(outcome.problem);
    if (outcome.problem === null) {
      await this.router.navigateByUrl('/');
    } else if (outcome.spent) {
      // The link opens nothing any more: back to the start, for a new one.
      if (this.emailed === 'recovery') {
        this.session.clearRecover();
      } else {
        this.session.clearSignup();
      }
    }
  }

  private async doing(act: () => Promise<string | null>): Promise<void> {
    this.busy.set(true);
    try {
      this.problem.set(await act());
    } catch (error: unknown) {
      this.problem.set(error instanceof ApiError ? error.message : unreached());
    } finally {
      this.busy.set(false);
    }
  }
}
