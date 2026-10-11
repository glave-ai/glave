import {
  DestroyRef,
  InjectionToken,
  type Signal,
  afterNextRender,
  computed,
  inject,
  signal,
} from '@angular/core';
import {
  type AbstractControl,
  type ControlValueAccessor,
  NgControl,
  Validators,
} from '@angular/forms';
import { FORM_FIELD } from '@angular/forms/signals';

/** What a form control tells the form field around it. */
export interface GlzFieldControl {
  /** Whether to show the control as invalid: invalid and touched. */
  readonly showError: Signal<boolean>;
  /** Messages for the errors signal forms report; reactive forms project `<glz-error>`. */
  readonly messages: Signal<readonly string[]>;
  readonly required: Signal<boolean>;
  /** Whether a `<label for>` can name the control (native inputs) or it needs `aria-labelledby`. */
  readonly labelable: boolean;
}

/** The form field around a control, as the control sees it. */
export interface GlzFieldHost {
  /** The id the control takes, so the label's `for` names it. */
  readonly id: string;
  readonly labelId: string;
  /** The id that describes the control now: its error, its hint, or none. */
  readonly describedBy: Signal<string | null>;
  register(control: GlzFieldControl): void;
}

/** Provided by `<glz-form-field>` for the control inside it. */
export const GLZ_FORM_FIELD = new InjectionToken<GlzFieldHost>('GLZ_FORM_FIELD');

/** The state of the form a control is bound to, from signal forms or reactive forms. */
export interface GlzFormState {
  readonly invalid: Signal<boolean>;
  readonly touched: Signal<boolean>;
  readonly required: Signal<boolean>;
  /** Error messages signal forms report; reactive forms leave messages to `<glz-error>`. */
  readonly messages: Signal<readonly string[]>;
}

const NO_MESSAGES: Signal<readonly string[]> = signal<readonly string[]>([]).asReadonly();

/**
 * Reads the form state of the control on this element: signal forms' `[formField]`, or a
 * reactive forms or `ngModel` directive. Without either, nothing is invalid. Call in an
 * injection context (a constructor or field initializer).
 */
export function injectFormState(): GlzFormState {
  const field = inject(FORM_FIELD, { self: true, optional: true });
  if (field !== null) {
    return {
      invalid: computed(() => field.state().invalid()),
      touched: computed(() => field.state().touched()),
      required: computed(() => field.state().required()),
      messages: computed(() =>
        field
          .errors()
          .map((error) => error.message)
          .filter((message): message is string => message !== undefined),
      ),
    };
  }
  const ngControl = inject(NgControl, { self: true, optional: true });
  const version = signal(0);
  if (ngControl !== null) {
    const destroyRef = inject(DestroyRef);
    // The control exists once the forms directive has set itself up: by the first render.
    afterNextRender(() => {
      const subscription = ngControl.control?.events.subscribe(() => {
        version.update((count) => count + 1);
      });
      version.update((count) => count + 1);
      destroyRef.onDestroy(() => subscription?.unsubscribe());
    });
  }
  const read = (pick: (control: AbstractControl) => boolean): Signal<boolean> =>
    computed(() => {
      version();
      const control = ngControl?.control;
      return control ? pick(control) : false;
    });
  return {
    invalid: read((control) => control.invalid),
    touched: read((control) => control.touched),
    // hasValidator compares by reference, so it takes the validator itself.
    // eslint-disable-next-line @typescript-eslint/unbound-method -- see above
    required: read((control) => control.hasValidator(Validators.required)),
    messages: NO_MESSAGES,
  };
}

/**
 * Makes `accessor` the value accessor of the reactive forms or `ngModel` directive on this
 * element, if there is one. Under signal forms' `[formField]` the control's `value` or
 * `checked` model is bound instead, so nothing is done. Call in a constructor.
 */
export function bindValueAccessor(accessor: ControlValueAccessor): void {
  const field = inject(FORM_FIELD, { self: true, optional: true });
  const ngControl = inject(NgControl, { self: true, optional: true });
  if (field === null && ngControl !== null) {
    ngControl.valueAccessor = accessor;
  }
}
