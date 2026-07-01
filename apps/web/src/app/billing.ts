import { DOCUMENT } from '@angular/common';
import { HttpClient, HttpErrorResponse } from '@angular/common/http';
import {
  Component,
  inject,
  Injectable,
  OnInit,
  output,
  signal,
} from '@angular/core';
import { FormControl, ReactiveFormsModule, Validators } from '@angular/forms';
import { ErrorStateMatcher } from '@angular/material/core';
import { MatButtonModule } from '@angular/material/button';
import { MatButtonToggleModule } from '@angular/material/button-toggle';
import { MatFormFieldModule } from '@angular/material/form-field';
import { MatInputModule } from '@angular/material/input';
import { firstValueFrom } from 'rxjs';

export type Role = 'owner' | 'member';
export type LimitPeriod = 'day' | 'month';

export interface BillingStatus {
  readonly account_id: string;
  readonly role: Role;
  readonly balance_usd: number;
  readonly org_limit_usd: number | null;
  readonly member_limit_usd: number | null;
}

/// The result of a checkout attempt. A provider that isn't wired up yet (503) is a
/// calm, expected outcome — not an error — so we model it as its own variant rather
/// than throwing (per the brand: transparent, never scary).
export type CheckoutResult =
  | { readonly kind: 'redirect'; readonly url: string }
  | { readonly kind: 'not_configured' };

/// Billing against the Glave backend. Same-origin, cookie-authed — the session
/// cookie (`glave_session`) rides along via `withCredentials`, mirroring AuthService.
@Injectable({ providedIn: 'root' })
export class BillingService {
  private readonly http = inject(HttpClient);

  /// Who's paying and where they stand: balance + any spend limit.
  async status(): Promise<BillingStatus> {
    return firstValueFrom(
      this.http.get<BillingStatus>('/billing/status', { withCredentials: true }),
    );
  }

  /// Set (or, with `null`, clear back to unlimited) the org-wide spend limit.
  async setLimit(amountUsd: number | null, period: LimitPeriod): Promise<void> {
    await firstValueFrom(
      this.http.post(
        '/billing/limit',
        { scope: 'org', amount_usd: amountUsd, period },
        { withCredentials: true },
      ),
    );
  }

  /// Start a credit top-up. On 200 we hand back the provider URL to redirect to;
  /// on 503 (no provider configured yet) we return `not_configured` so the UI can
  /// say "coming soon" instead of showing an error.
  async checkout(amountUsd: number): Promise<CheckoutResult> {
    try {
      const res = await firstValueFrom(
        this.http.post<{ readonly url: string }>(
          '/billing/checkout',
          { amount_usd: amountUsd },
          { withCredentials: true },
        ),
      );
      return { kind: 'redirect', url: res.url };
    } catch (error) {
      if (error instanceof HttpErrorResponse && error.status === 503) {
        return { kind: 'not_configured' };
      }
      throw error;
    }
  }
}

/// Show a limit error only once the field's been touched and is actually wrong —
/// warm, not scolding (mirrors login's matcher).
class TouchedErrorMatcher implements ErrorStateMatcher {
  isErrorState(control: FormControl | null): boolean {
    return !!control && control.invalid && control.touched;
  }
}

const CREDIT_PACKS: readonly number[] = [25, 50, 100];

@Component({
  selector: 'app-billing-panel',
  imports: [
    ReactiveFormsModule,
    MatButtonModule,
    MatButtonToggleModule,
    MatFormFieldModule,
    MatInputModule,
  ],
  template: `
    <main class="billing">
      <header class="head">
        <button
          type="button"
          class="back"
          (click)="dismissed.emit()"
          aria-label="Back to chat"
        >
          ← Back to chat
        </button>
        <h1 class="title">Credits &amp; spending</h1>
        <p class="lede">
          What you spend, what's left, and the limit you're comfortable with — all in
          one calm place.
        </p>
      </header>

      @if (loading()) {
        <p class="loading" role="status" aria-live="polite">Getting your balance…</p>
      } @else if (status(); as s) {
        <section class="panel" aria-labelledby="balance-heading">
          <h2 id="balance-heading" class="panel__label">Balance</h2>
          <p class="balance">{{ formatUsd(s.balance_usd) }} <span class="balance__unit">in credits</span></p>
        </section>

        <section class="panel" aria-labelledby="limit-heading">
          <h2 id="limit-heading" class="panel__label">Spending limit</h2>
          <p class="limit-current">
            Right now:
            <strong>{{ limitLabel(s) }}</strong>
          </p>

          @if (s.role === 'owner') {
            <div class="limit-form">
              <mat-form-field appearance="outline" class="limit-form__amount">
                <mat-label>Limit</mat-label>
                <span matTextPrefix>$&nbsp;</span>
                <input
                  matInput
                  type="number"
                  inputmode="decimal"
                  min="0"
                  step="1"
                  autocomplete="off"
                  [formControl]="amount"
                  [errorStateMatcher]="matcher"
                />
                @if (amount.touched && amount.invalid) {
                  <mat-error>Enter an amount of $0 or more.</mat-error>
                }
              </mat-form-field>

              <mat-button-toggle-group
                class="limit-form__period"
                [formControl]="period"
                aria-label="Limit period"
              >
                <mat-button-toggle value="month">Monthly</mat-button-toggle>
                <mat-button-toggle value="day">Daily</mat-button-toggle>
              </mat-button-toggle-group>

              <button
                mat-flat-button
                type="button"
                class="limit-form__save"
                [disabled]="saving() || amount.invalid"
                (click)="saveLimit()"
              >
                {{ saving() ? 'Saving…' : 'Save limit' }}
              </button>
            </div>

            <button
              type="button"
              class="clear"
              [disabled]="saving() || s.org_limit_usd === null"
              (click)="clearLimit()"
            >
              Clear limit (no cap)
            </button>
          } @else {
            <p class="hint">Only the account owner can change the spending limit.</p>
          }
        </section>

        <section class="panel" aria-labelledby="topup-heading">
          <h2 id="topup-heading" class="panel__label">Add credits</h2>
          <p class="hint">Top up whenever you like — credits never expire.</p>
          <div class="packs" role="group" aria-label="Credit packs">
            @for (pack of packs; track pack) {
              <button
                mat-stroked-button
                type="button"
                class="pack"
                [disabled]="buying() !== null"
                (click)="buy(pack)"
              >
                @if (buying() === pack) {
                  Starting…
                } @else {
                  Add {{ formatUsd(pack) }}
                }
              </button>
            }
          </div>
        </section>

        <p class="status" role="status" aria-live="polite">
          @if (notice()) {
            <span class="notice">{{ notice() }}</span>
          }
          @if (error()) {
            <span class="error">{{ error() }}</span>
          }
        </p>
      } @else {
        <p class="status error" role="status" aria-live="polite">
          {{ error() || 'We couldn’t load your billing just now.' }}
        </p>
      }
    </main>
  `,
  styles: `
    :host {
      display: block;
      min-height: 100dvh;
    }

    .billing {
      display: flex;
      flex-direction: column;
      gap: 1.75rem;
      max-width: 640px;
      margin: 0 auto;
      min-height: 100dvh;
      padding: 2.5rem 1.25rem 3rem;
      box-sizing: border-box;
    }

    .head {
      display: flex;
      flex-direction: column;
      gap: 0.5rem;
    }

    .back {
      align-self: flex-start;
      margin-bottom: 0.5rem;
      padding: 0.35rem 0;
      border: none;
      background: transparent;
      color: var(--mat-sys-on-surface-variant);
      font: inherit;
      cursor: pointer;
    }

    .back:hover {
      color: var(--mat-sys-on-surface);
    }

    .title {
      margin: 0;
      font-family: ui-serif, Georgia, 'Times New Roman', serif;
      font-size: 1.75rem;
      font-weight: 500;
      color: var(--mat-sys-on-surface);
    }

    .lede {
      margin: 0;
      line-height: 1.6;
      color: var(--mat-sys-on-surface-variant);
    }

    .loading {
      margin: 0;
      padding: 2rem 0;
      color: var(--mat-sys-on-surface-variant);
    }

    .panel {
      display: flex;
      flex-direction: column;
      gap: 0.85rem;
      padding: 1.5rem;
      border: 1px solid var(--mat-sys-outline-variant);
      border-radius: 18px;
      background: var(--mat-sys-surface-container-low);
    }

    .panel__label {
      margin: 0;
      font-size: 0.85rem;
      font-weight: 600;
      letter-spacing: 0.04em;
      text-transform: uppercase;
      color: var(--mat-sys-on-surface-variant);
    }

    .balance {
      margin: 0;
      font-family: ui-serif, Georgia, 'Times New Roman', serif;
      font-size: 2.25rem;
      font-weight: 500;
      color: var(--mat-sys-on-surface);
    }

    .balance__unit {
      font-family: var(--mat-sys-body-large-font, inherit);
      font-size: 1rem;
      color: var(--mat-sys-on-surface-variant);
    }

    .limit-current {
      margin: 0;
      color: var(--mat-sys-on-surface);
    }

    .limit-form {
      display: flex;
      flex-wrap: wrap;
      align-items: flex-start;
      gap: 0.75rem;
    }

    .limit-form__amount {
      flex: 1 1 8rem;
      min-width: 8rem;
    }

    .limit-form__period {
      flex: 0 0 auto;
      height: 56px;
    }

    .limit-form__save {
      height: 56px;
    }

    .clear {
      align-self: flex-start;
      padding: 0.35rem 0;
      border: none;
      background: transparent;
      color: var(--mat-sys-on-surface-variant);
      font: inherit;
      text-decoration: underline;
      cursor: pointer;
    }

    .clear:hover:not(:disabled) {
      color: var(--mat-sys-on-surface);
    }

    .clear:disabled {
      opacity: 0.5;
      cursor: default;
      text-decoration: none;
    }

    .hint {
      margin: 0;
      color: var(--mat-sys-on-surface-variant);
    }

    .packs {
      display: flex;
      flex-wrap: wrap;
      gap: 0.75rem;
    }

    .pack {
      flex: 1 1 auto;
      min-width: 8rem;
      height: 52px;
    }

    .status {
      min-height: 1.5rem;
      margin: 0;
      text-align: center;
    }

    .notice {
      color: var(--mat-sys-on-surface-variant);
    }

    .error {
      color: var(--mat-sys-error);
    }
  `,
})
export class BillingPanel implements OnInit {
  private readonly billing = inject(BillingService);
  private readonly doc = inject(DOCUMENT);

  /// Emitted when the user asks to return to the chat.
  readonly dismissed = output<void>();

  protected readonly packs = CREDIT_PACKS;
  protected readonly matcher = new TouchedErrorMatcher();

  protected readonly status = signal<BillingStatus | null>(null);
  protected readonly loading = signal(true);
  protected readonly saving = signal(false);
  // Which pack is mid-checkout (its dollar amount), or null when idle.
  protected readonly buying = signal<number | null>(null);
  protected readonly error = signal('');
  protected readonly notice = signal('');

  protected readonly amount = new FormControl<number | null>(null, {
    validators: [Validators.min(0)],
  });
  protected readonly period = new FormControl<LimitPeriod>('month', {
    nonNullable: true,
  });

  async ngOnInit(): Promise<void> {
    await this.load();
  }

  private async load(): Promise<void> {
    this.loading.set(true);
    try {
      const s = await this.billing.status();
      this.status.set(s);
      // Seed the form from the current limit so "Save" reflects what they see. The
      // backend doesn't echo the period on status, so default to month (the common
      // case) rather than guessing.
      this.amount.setValue(s.org_limit_usd);
      this.period.setValue('month');
    } catch {
      // A 401 is handled by the app auth gate; anything else is a gentle failure.
      this.status.set(null);
      this.error.set('We couldn’t load your billing just now.');
    } finally {
      this.loading.set(false);
    }
  }

  protected async saveLimit(): Promise<void> {
    this.amount.markAsTouched();
    if (this.amount.invalid || this.saving()) {
      return;
    }
    const value = this.amount.value;
    await this.applyLimit(value === null ? null : Number(value));
  }

  protected async clearLimit(): Promise<void> {
    if (this.saving()) {
      return;
    }
    await this.applyLimit(null);
  }

  private async applyLimit(amountUsd: number | null): Promise<void> {
    this.saving.set(true);
    this.error.set('');
    this.notice.set('');
    try {
      await this.billing.setLimit(amountUsd, this.period.value);
      // Optimistic-but-verified: re-fetch so the UI shows the server's truth.
      await this.load();
      this.notice.set(
        amountUsd === null ? 'Limit cleared — no cap.' : 'Limit saved.',
      );
    } catch {
      this.error.set('That limit didn’t save. Mind trying again?');
    } finally {
      this.saving.set(false);
    }
  }

  protected async buy(amountUsd: number): Promise<void> {
    if (this.buying() !== null) {
      return;
    }
    this.buying.set(amountUsd);
    this.error.set('');
    this.notice.set('');
    try {
      const result = await this.billing.checkout(amountUsd);
      if (result.kind === 'redirect') {
        const view = this.doc.defaultView;
        if (view) {
          view.location.href = result.url;
        }
        return;
      }
      this.notice.set('Credit top-ups are coming soon.');
    } catch {
      this.error.set('We couldn’t start that top-up. Please try again.');
    } finally {
      this.buying.set(null);
    }
  }

  protected formatUsd(amount: number): string {
    return new Intl.NumberFormat('en-US', {
      style: 'currency',
      currency: 'USD',
    }).format(amount);
  }

  protected limitLabel(s: BillingStatus): string {
    if (s.org_limit_usd === null) {
      return 'No limit';
    }
    // The backend doesn't echo the period on status, so we label with the amount
    // (the load-bearing fact) and let the form's selector carry the cadence.
    return `${this.formatUsd(s.org_limit_usd)} per period`;
  }
}
