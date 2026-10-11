import { ChangeDetectionStrategy, Component, computed, inject, signal } from '@angular/core';
import { toSignal } from '@angular/core/rxjs-interop';
import { NonNullableFormBuilder, ReactiveFormsModule, Validators } from '@angular/forms';
import { FormField, email, form, required } from '@angular/forms/signals';
import {
  GlzButtonComponent,
  GlzCheckboxComponent,
  GlzComboboxComponent,
  GlzErrorComponent,
  GlzFormFieldComponent,
  GlzInputDirective,
  GlzRadioComponent,
  GlzRadioGroupComponent,
  GlzSliderDirective,
  GlzSwitchComponent,
  GlzToastService,
  type GlzOption,
} from '@glave/glaze';
import { map } from 'rxjs';

const COPY = {
  heading: 'Forms',
  reactive: 'Reactive forms',
  signal: 'Signal forms',
  name: 'Space name',
  nameHint: 'Everyone in the space sees it',
  nameMissing: 'Give the space a name.',
  notes: 'Notes',
  notesHint: 'The box grows as you write',
  region: 'Region',
  due: 'Due',
  budget: 'Budget',
  model: 'Model',
  modelPlaceholder: 'Search models',
  approval: 'Approval',
  ask: 'Ask every time',
  auto: 'Auto: the judge decides',
  never: 'Never ask',
  memory: 'Remember things between threads',
  policy: 'Policy',
  terms: 'I have read the policy',
  termsMissing: 'Read and accept the policy to go on.',
  create: 'Create the space',
  created: 'Space created',
  email: 'Email',
  emailHint: 'Signal forms show their own messages',
  save: 'Save',
  saved: 'Saved',
} as const;

const REGIONS: readonly GlzOption[] = [
  { value: 'local', label: 'This computer' },
  { value: 'eu', label: 'Europe' },
  { value: 'us', label: 'United States' },
];

const MODELS: readonly GlzOption[] = [
  { value: 'hope', label: 'Hope' },
  { value: 'haiku', label: 'Claude Haiku' },
  { value: 'luna', label: 'GPT-6 Luna' },
  { value: 'flash', label: 'DeepSeek V4.1 Flash' },
];

const APPROVALS: readonly GlzOption[] = [
  { value: 'ask', label: COPY.ask },
  { value: 'auto', label: COPY.auto },
  { value: 'never', label: COPY.never },
];

/** The showcase's form controls, under reactive forms and under signal forms. */
@Component({
  selector: 'app-showcase-forms',
  imports: [
    ReactiveFormsModule,
    FormField,
    GlzButtonComponent,
    GlzCheckboxComponent,
    GlzComboboxComponent,
    GlzErrorComponent,
    GlzFormFieldComponent,
    GlzInputDirective,
    GlzRadioComponent,
    GlzRadioGroupComponent,
    GlzSliderDirective,
    GlzSwitchComponent,
  ],
  templateUrl: './showcase-forms.component.html',
  styleUrl: './showcase-forms.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class ShowcaseFormsComponent {
  protected readonly copy = COPY;
  protected readonly regions = REGIONS;
  protected readonly models = MODELS;
  protected readonly approvals = APPROVALS;
  protected readonly values = { notesRows: 2, budgetMin: 0, budgetMax: 100 } as const;
  private readonly toasts = inject(GlzToastService);

  private readonly forms = inject(NonNullableFormBuilder);
  // Validators are passed as themselves: forms (and Glaze's required mark) compare them by
  // reference.
  /* eslint-disable @typescript-eslint/unbound-method -- see above */
  protected readonly space = this.forms.group({
    name: this.forms.control('', Validators.required),
    notes: this.forms.control(''),
    region: this.forms.control('local'),
    due: this.forms.control(''),
    budget: this.forms.control(50),
    model: this.forms.control<string | null>('hope'),
    approval: this.forms.control<string | null>('ask'),
    memory: this.forms.control(true),
    terms: this.forms.control(false, Validators.requiredTrue),
  });
  /* eslint-enable @typescript-eslint/unbound-method */
  private readonly status = toSignal(this.space.events.pipe(map(() => this.space.controls)), {
    initialValue: this.space.controls,
  });
  protected readonly nameMissing = computed(() => this.status().name.hasError('required'));
  protected readonly termsMissing = computed(() => this.status().terms.hasError('required'));
  protected readonly budgetText = computed(
    () => `${String(this.status().budget.value)}% of the budget`,
  );

  protected readonly contact = signal({ email: '' });
  protected readonly contactForm = form(this.contact, (path) => {
    required(path.email, { message: 'An email address, please.' });
    email(path.email, { message: 'That is not an email address.' });
  });

  protected create(): void {
    this.space.markAllAsTouched();
    if (this.space.valid) {
      this.toasts.show(COPY.created, { tone: 'success' });
    }
  }

  protected save(): void {
    this.contactForm.email().markAsTouched();
    if (this.contactForm().valid()) {
      this.toasts.show(COPY.saved, { tone: 'success' });
    }
  }
}
