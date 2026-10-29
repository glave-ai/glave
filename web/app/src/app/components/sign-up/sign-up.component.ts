import { ChangeDetectionStrategy, Component, computed, inject, signal } from '@angular/core';
import { Router, RouterLink } from '@angular/router';
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

/**
 * Signing up, where the server takes sign-ups: the person's email, then a link the server
 * sends there, then, opened from it, their first passkey, which makes their account and signs
 * them in. Nothing here speaks of organizations: theirs is made with the account.
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
  protected readonly copy = {
    name: 'glave',
    emailHeading: $localize`:@@signUp.emailHeading:Get started with glave`,
    emailText: $localize`:@@signUp.emailText:We'll send you a link. Your email is only for getting back in and for invites, never for ads.`,
    email: $localize`:@@signUp.email:Email`,
    send: $localize`:@@signUp.send:Send me a link`,
    sending: $localize`:@@signUp.sending:Sending…`,
    have: $localize`:@@signUp.have:Already have an account?`,
    signIn: $localize`:@@signUp.signIn:Sign in with your passkey`,
    sentHeading: $localize`:@@signUp.sentHeading:Check your email`,
    again: $localize`:@@signUp.again:Send it again`,
    passkeyHeading: $localize`:@@signUp.passkeyHeading:Make your passkey`,
    passkeyText: $localize`:@@signUp.passkeyText:This is how you'll sign in: your fingerprint, face or device code. No password.`,
    make: $localize`:@@signUp.make:Make a passkey`,
    waiting: $localize`:@@signUp.waiting:Waiting for your device…`,
    then: $localize`:@@signUp.then:Then you're in. Add a model and start a chat.`,
  } as const;
  protected readonly icons = { mail: Mail, key: KeyRound } as const;
  protected readonly signInLink = '/sign-in';
  protected readonly email = signal('');
  private readonly sentTo = signal<string | null>(null);
  protected readonly busy = signal(false);
  protected readonly problem = signal<string | null>(null);

  protected readonly step = computed<Step>(() => {
    if (this.session.signupCode() !== null) {
      return 'passkey';
    }
    return this.sentTo() === null ? 'email' : 'sent';
  });
  protected readonly sentText = computed(() => {
    const to = this.sentTo() ?? '';
    return $localize`:@@signUp.sentText:We sent a link to ${to}:EMAIL:. Open it on this device. It works for 15 minutes.`;
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
      await this.session.call('signup/start', { email });
      this.sentTo.set(email);
      return null;
    });
  }

  /** Sends the link again, to the same email. */
  protected async again(): Promise<void> {
    await this.send();
  }

  /** Makes the first passkey with the link the tab was opened with, then opens glave. */
  protected async make(): Promise<void> {
    const code = this.session.signupCode();
    if (this.busy() || code === null) {
      return;
    }
    this.busy.set(true);
    const problem = await this.passkeys.signUp(code);
    this.busy.set(false);
    this.problem.set(problem);
    if (problem === null) {
      await this.router.navigateByUrl('/');
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
