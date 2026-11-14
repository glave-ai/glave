import {
  ChangeDetectionStrategy,
  Component,
  LOCALE_ID,
  computed,
  effect,
  inject,
  model,
  signal,
} from '@angular/core';
import {
  GlzButtonComponent,
  GlzDialogComponent,
  GlzFormFieldComponent,
  GlzInputDirective,
} from '@glave/glaze';

import { plural } from '../../lib/plural';
import { OrganizationsService } from '../../services/organizations.service';

/** The longest name an organization takes, as its brand's name. */
const NAME_LIMIT = 80;

/**
 * Creating an organization, to invite people to: its name, then the person works in it. A name
 * the server refuses, or an organization past the most one person creates, says why here.
 */
@Component({
  selector: 'app-new-organization-dialog',
  imports: [GlzButtonComponent, GlzDialogComponent, GlzFormFieldComponent, GlzInputDirective],
  templateUrl: './new-organization-dialog.component.html',
  styleUrl: './new-organization-dialog.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class NewOrganizationDialogComponent {
  /** Whether the dialog is open. */
  readonly open = model(false);

  private readonly organizations = inject(OrganizationsService);
  private readonly locale = inject(LOCALE_ID);
  protected readonly copy = {
    heading: $localize`:@@newOrganization.heading:New organization`,
    name: $localize`:@@newOrganization.name:Name`,
    note: $localize`:@@newOrganization.note:You can invite people to it once it's made.`,
    cancel: $localize`:@@newOrganization.cancel:Cancel`,
    create: $localize`:@@newOrganization.create:Create`,
    creating: $localize`:@@newOrganization.creating:Creating…`,
  } as const;
  protected readonly limit = NAME_LIMIT;
  protected readonly hint = plural(
    $localize`:@@newOrganization.hint:{${NAME_LIMIT}:LIMIT:, plural, one {Up to # character} other {Up to # characters}}`,
    this.locale,
  );
  protected readonly draft = signal('');
  protected readonly busy = signal(false);
  protected readonly problem = signal<string | null>(null);
  private readonly trimmed = computed(() => this.draft().trim());
  protected readonly invalid = computed(
    () => this.trimmed() === '' || this.trimmed().length > NAME_LIMIT,
  );
  protected readonly createLabel = computed(() =>
    this.busy() ? this.copy.creating : this.copy.create,
  );

  constructor() {
    // Each time it opens, it starts empty.
    effect(() => {
      if (this.open()) {
        this.draft.set('');
        this.problem.set(null);
      }
    });
  }

  protected typed(event: Event): void {
    if (event.target instanceof HTMLInputElement) {
      this.draft.set(event.target.value);
    }
  }

  /** Creates it, then works in it; or says why it was not made. */
  protected async create(event?: Event): Promise<void> {
    event?.preventDefault();
    if (this.invalid() || this.busy()) {
      return;
    }
    this.busy.set(true);
    const refused = await this.organizations.create(this.trimmed());
    this.busy.set(false);
    this.problem.set(refused);
    if (refused === null) {
      this.open.set(false);
    }
  }

  protected cancel(): void {
    this.open.set(false);
  }
}
