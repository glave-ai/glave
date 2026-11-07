import { ChangeDetectionStrategy, Component, computed, input, output } from '@angular/core';
import type { ConnectorAccount } from '@glave/client';
import { GlzMenuButtonComponent, type GlzOption } from '@glave/glaze';

/** What the person picked from an account's menu. */
export type AccountAction =
  | { readonly kind: 'place'; readonly place: string }
  | { readonly kind: 'again' }
  | { readonly kind: 'disconnect' };

/** The menu's value for signing in again and for disconnecting, beside each place's own. */
const AGAIN = '\u0000again';
const DISCONNECT = '\u0000disconnect';

/**
 * One account of a connected service, beneath the service on the Connections page when it has
 * two or more: its name, where it is used in the server's words, and its own menu, which moves
 * it to another of the person's places, signs in to it again when its sign-in no longer works,
 * or disconnects it.
 */
@Component({
  selector: 'app-connector-account',
  imports: [GlzMenuButtonComponent],
  templateUrl: './connector-account.component.html',
  styleUrl: './connector-account.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
  host: { 'data-testid': 'connector-account', role: 'listitem' },
})
export class ConnectorAccountComponent {
  /** The service's own name, which an account never named goes by: `Stripe`. */
  readonly service = input.required<string>();
  readonly account = input.required<ConnectorAccount>();
  /** Where the person may use it, each place's value and words, with its own among them. */
  readonly places = input<readonly GlzOption[]>([]);
  /** Its place's value: `everywhere`, or the project its threads alone use. */
  readonly place = input.required<string>();
  /** Picked from its menu. */
  readonly picked = output<AccountAction>();

  protected readonly copy = {
    again: $localize`:@@connectorAccount.again:Sign in again`,
    disconnect: $localize`:@@connectorAccount.disconnect:Disconnect`,
  } as const;
  /** What the account goes by: its name, or the service's for the one never named. */
  protected readonly name = computed(() => this.account().name ?? this.service());
  protected readonly menuLabel = computed(() => {
    const service = this.service();
    const name = this.name();
    return $localize`:@@connectorAccount.menu:Options for ${service}:SERVICE: ${name}:NAME:`;
  });
  /** Each other place it may move to, then signing in again when it needs it, then disconnecting. */
  protected readonly actions = computed((): readonly GlzOption[] => [
    ...this.places().filter((it) => it.value !== this.place()),
    ...(this.account().status === 'sign_in_again'
      ? [{ value: AGAIN, label: this.copy.again }]
      : []),
    { value: DISCONNECT, label: this.copy.disconnect },
  ]);

  protected act(value: string): void {
    if (value === AGAIN) {
      this.picked.emit({ kind: 'again' });
    } else if (value === DISCONNECT) {
      this.picked.emit({ kind: 'disconnect' });
    } else {
      this.picked.emit({ kind: 'place', place: value });
    }
  }
}
