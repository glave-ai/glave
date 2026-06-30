import {
  AfterViewInit,
  Component,
  ElementRef,
  inject,
  signal,
  viewChild,
} from '@angular/core';
import { FormControl, ReactiveFormsModule, Validators } from '@angular/forms';
import { ErrorStateMatcher } from '@angular/material/core';
import { MatButtonModule } from '@angular/material/button';
import { MatFormFieldModule } from '@angular/material/form-field';
import { MatInputModule } from '@angular/material/input';
import { AuthService } from './auth';

type Step = 'email' | 'code';

/**
 * Show an error only once the field has been touched and is actually wrong — never
 * on a pristine/empty field, and never just because the form was submitted. Warm,
 * not scolding (the brand): we don't flag you before you've had a fair go.
 */
class TouchedErrorMatcher implements ErrorStateMatcher {
  isErrorState(control: FormControl | null): boolean {
    return !!control && control.invalid && control.touched;
  }
}

@Component({
  selector: 'app-login',
  imports: [ReactiveFormsModule, MatFormFieldModule, MatInputModule, MatButtonModule],
  template: `
    <main class="welcome">
      <div class="presence" aria-hidden="true">
        <div class="orb"></div>
      </div>

      @if (step() === 'email') {
        <form class="card" (ngSubmit)="sendCode()">
          <h1 class="hi">Hi, I'm Hope.</h1>
          <p class="lede">What's your email? I'll send you a code to get you in.</p>

          <mat-form-field appearance="outline" class="field">
            <mat-label>Your email</mat-label>
            <input
              matInput
              #emailInput
              type="email"
              autocomplete="email"
              inputmode="email"
              [formControl]="email"
              [errorStateMatcher]="matcher"
              (keyup.enter)="sendCode()"
            />
            @if (email.touched && email.invalid) {
              <mat-error>Could you double-check that email?</mat-error>
            }
          </mat-form-field>

          <button
            mat-flat-button
            type="submit"
            class="go"
            [disabled]="sending() || email.invalid"
          >
            {{ sending() ? 'Sending…' : 'Send my code' }}
          </button>
        </form>
      } @else {
        <form class="card" (ngSubmit)="signIn()">
          <h1 class="hi">Check your inbox.</h1>
          <p class="lede">
            I sent a 6-digit code to <strong>{{ emailValue() }}</strong>. Pop it in below.
          </p>

          @if (devCode()) {
            <p class="dev-hint">
              Dev shortcut — your code is <code>{{ devCode() }}</code>.
            </p>
          }

          <mat-form-field appearance="outline" class="field">
            <mat-label>6-digit code</mat-label>
            <input
              matInput
              #codeInput
              type="text"
              autocomplete="one-time-code"
              inputmode="numeric"
              maxlength="6"
              [formControl]="code"
              [errorStateMatcher]="matcher"
              (keyup.enter)="signIn()"
            />
            @if (code.touched && code.invalid) {
              <mat-error>That should be the 6 digits I sent you.</mat-error>
            }
          </mat-form-field>

          <button
            mat-flat-button
            type="submit"
            class="go"
            [disabled]="sending() || code.invalid"
          >
            {{ sending() ? 'Letting you in…' : 'Come on in' }}
          </button>

          <button type="button" class="back" (click)="backToEmail()">
            Use a different email
          </button>
        </form>
      }

      <p class="status" role="status" aria-live="polite">
        @if (error()) {
          <span class="error">{{ error() }}</span>
        }
      </p>
    </main>
  `,
  styles: `
    :host {
      display: block;
      min-height: 100dvh;
    }

    .welcome {
      display: flex;
      flex-direction: column;
      align-items: center;
      max-width: 420px;
      margin: 0 auto;
      min-height: 100dvh;
      padding: 0 1.5rem 2rem;
      box-sizing: border-box;
    }

    .presence {
      display: flex;
      justify-content: center;
      padding: 4rem 0 1.5rem;
    }

    .orb {
      width: 108px;
      height: 108px;
      border-radius: 50%;
      background: radial-gradient(
        circle at 50% 45%,
        rgba(255, 247, 224, 0.95) 0%,
        rgba(247, 211, 132, 0.82) 32%,
        rgba(239, 177, 60, 0.45) 60%,
        rgba(239, 177, 60, 0) 80%
      );
      filter: blur(4px);
      animation: breathe 6s ease-in-out infinite;
    }

    @keyframes breathe {
      0%,
      100% {
        transform: scale(1);
        opacity: 0.85;
      }
      50% {
        transform: scale(1.05);
        opacity: 1;
      }
    }

    .card {
      display: flex;
      flex-direction: column;
      gap: 1rem;
      width: 100%;
      text-align: center;
    }

    .hi {
      margin: 0;
      font-family: ui-serif, Georgia, 'Times New Roman', serif;
      font-size: 1.75rem;
      font-weight: 500;
      color: var(--mat-sys-on-surface);
    }

    .lede {
      margin: 0 0 0.5rem;
      line-height: 1.6;
      color: var(--mat-sys-on-surface-variant);
    }

    .dev-hint {
      margin: 0 0 0.25rem;
      font-size: 0.875rem;
      color: var(--mat-sys-on-surface-variant);
    }

    .dev-hint code {
      padding: 0.1rem 0.4rem;
      border-radius: 6px;
      background: var(--mat-sys-primary-container);
      color: var(--mat-sys-on-primary-container);
      font-weight: 600;
      letter-spacing: 0.1em;
    }

    .field {
      width: 100%;
      text-align: start;
    }

    .go {
      height: 52px;
      font-size: 1rem;
    }

    .back {
      margin-top: 0.25rem;
      border: none;
      background: transparent;
      color: var(--mat-sys-on-surface-variant);
      font: inherit;
      text-decoration: underline;
      cursor: pointer;
    }

    .back:hover {
      color: var(--mat-sys-on-surface);
    }

    .status {
      min-height: 1.5rem;
      margin: 1rem 0 0;
      text-align: center;
    }

    .error {
      color: var(--mat-sys-error);
    }

    @media (prefers-reduced-motion: reduce) {
      .orb {
        animation: none;
      }
    }
  `,
})
export class Login implements AfterViewInit {
  private readonly auth = inject(AuthService);

  private readonly emailInput = viewChild<ElementRef<HTMLInputElement>>('emailInput');
  private readonly codeInput = viewChild<ElementRef<HTMLInputElement>>('codeInput');

  protected readonly step = signal<Step>('email');
  protected readonly sending = signal(false);
  protected readonly error = signal('');
  protected readonly devCode = signal<string | null>(null);
  protected readonly emailValue = signal('');
  protected readonly matcher = new TouchedErrorMatcher();

  protected readonly email = new FormControl('', {
    nonNullable: true,
    validators: [Validators.required, Validators.email],
  });
  protected readonly code = new FormControl('', {
    nonNullable: true,
    validators: [Validators.required, Validators.pattern(/^\d{6}$/)],
  });

  ngAfterViewInit(): void {
    this.emailInput()?.nativeElement.focus();
  }

  protected async sendCode(): Promise<void> {
    const address = this.email.value.trim();
    // An empty field gets a gentle nudge (focus), never a red error.
    if (!address) {
      this.emailInput()?.nativeElement.focus();
      return;
    }
    this.email.markAsTouched();
    if (this.email.invalid || this.sending()) {
      return;
    }
    this.sending.set(true);
    this.error.set('');
    try {
      const { devCode } = await this.auth.startOtp(address);
      this.emailValue.set(address);
      this.devCode.set(devCode);
      if (devCode) {
        this.code.setValue(devCode);
      }
      this.step.set('code');
      // Wait a tick so the code field renders before we move focus to it.
      queueMicrotask(() => this.codeInput()?.nativeElement.focus());
    } catch {
      this.error.set("I couldn't send that code just now. Mind trying again?");
    } finally {
      this.sending.set(false);
    }
  }

  protected async signIn(): Promise<void> {
    if (!this.code.value.trim()) {
      this.codeInput()?.nativeElement.focus();
      return;
    }
    this.code.markAsTouched();
    if (this.code.invalid || this.sending()) {
      return;
    }
    this.sending.set(true);
    this.error.set('');
    try {
      await this.auth.verifyOtp(this.emailValue(), this.code.value.trim());
    } catch {
      this.error.set("That code didn't match. Check it and try once more?");
    } finally {
      this.sending.set(false);
    }
  }

  protected backToEmail(): void {
    this.error.set('');
    this.devCode.set(null);
    this.code.reset('');
    this.step.set('email');
    queueMicrotask(() => this.emailInput()?.nativeElement.focus());
  }
}
