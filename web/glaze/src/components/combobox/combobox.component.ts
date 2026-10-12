import { ActiveDescendantKeyManager, type Highlightable } from '@angular/cdk/a11y';
import {
  ChangeDetectionStrategy,
  Component,
  DestroyRef,
  type ElementRef,
  Injector,
  booleanAttribute,
  computed,
  effect,
  inject,
  input,
  model,
  output,
  signal,
  viewChild,
} from '@angular/core';
import type { ControlValueAccessor } from '@angular/forms';
import type { FormValueControl } from '@angular/forms/signals';

import { GLZ_FORM_FIELD, bindValueAccessor, injectFormState } from '../../tokens/forms';
import { injectId } from '../../tokens/ids';
import { type GlzOption, filterOptions, labelOf } from '../../lib/options';
import { positionPopover } from '../../lib/popover';
import { testIds } from '../../lib/test-ids';

/** The parts end-to-end tests reach, as `<testId>-<part>`. */
const PARTS = { input: 'input', listbox: 'listbox', option: 'option' } as const;

/** An option as the list shows it. */
interface ShownOption extends GlzOption {
  readonly id: string;
  readonly selected: boolean;
  readonly active: boolean;
}

/**
 * A text input that suggests options as you type and picks one: the ARIA combobox pattern
 * with a listbox popup. The keyboard model is the CDK's (ActiveDescendantKeyManager): arrow
 * keys move through the suggestions, Home and End jump, Enter picks, Escape closes. The list
 * is a native popover placed under the input. The value is the picked option's `value`;
 * binds with reactive forms, `ngModel`, signal forms or `[(value)]`.
 *
 * ```html
 * <glz-form-field label="Model">
 *   <glz-combobox [options]="models" [(value)]="model" placeholder="Search models" />
 * </glz-form-field>
 * ```
 */
@Component({
  selector: 'glz-combobox',
  templateUrl: './combobox.component.html',
  styleUrl: './combobox.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
  host: { '[attr.data-testid]': 'testId()' },
})
export class GlzComboboxComponent implements ControlValueAccessor, FormValueControl<string | null> {
  /** The test hook for this component; its parts get `<testId>-<part>`. */
  readonly testId = input<string | null>(null);
  protected readonly ids = computed(() => testIds(this.testId(), PARTS));

  /** The picked option's value, or `null`. */
  readonly value = model<string | null>(null);
  readonly options = input<readonly GlzOption[]>([]);
  readonly disabled = input(false, { transform: booleanAttribute });
  readonly placeholder = input('');
  /** The input's name when no form field labels it. */
  readonly label = input('');
  /** Shown in the list when nothing matches. */
  readonly emptyText = input('No matches');
  /** Emitted when focus leaves the input: it has been touched. */
  readonly touch = output();

  private readonly field = inject(GLZ_FORM_FIELD, { optional: true });
  private readonly state = injectFormState();
  private readonly formDisabled = signal(false);
  private onChanged: (value: string | null) => void = () => undefined;
  private onTouched: () => void = () => undefined;

  protected readonly listboxId = injectId('glz-combobox-list');
  protected readonly inputId = computed(() => this.field?.id ?? null);
  /** The text in the input. */
  protected readonly query = signal('');
  protected readonly open = signal(false);
  private readonly activeIndex = signal(-1);
  /** Whether the person has typed since the list opened, so the list filters. */
  private readonly typed = signal(false);
  private readonly input = viewChild.required<ElementRef<HTMLInputElement>>('input');
  private readonly listbox = viewChild.required<ElementRef<HTMLElement>>('listbox');

  protected readonly isDisabled = computed(() => this.disabled() || this.formDisabled());
  readonly showError = computed(() => this.state.invalid() && this.state.touched());
  private readonly matches = computed(() =>
    this.typed() ? filterOptions(this.options(), this.query()) : this.options(),
  );
  protected readonly shown = computed<readonly ShownOption[]>(() =>
    this.matches().map((option, index) => ({
      ...option,
      id: `${this.listboxId}-${String(index)}`,
      selected: option.value === this.value(),
      active: index === this.activeIndex(),
    })),
  );
  protected readonly activeId = computed(() =>
    this.open() ? (this.shown().find((option) => option.active)?.id ?? null) : null,
  );
  protected readonly ariaLabel = computed(() =>
    this.field === null && this.label() !== '' ? this.label() : null,
  );
  protected readonly describedBy = computed(() => this.field?.describedBy() ?? null);
  protected readonly ariaInvalid = computed(() => (this.showError() ? 'true' : null));
  protected readonly ariaRequired = computed(() => (this.state.required() ? 'true' : null));

  /** The CDK's keyboard model over the shown options; the active one is shown by id only. */
  private readonly highlightables = computed<readonly Highlightable[]>(() =>
    this.matches().map(() => ({
      setActiveStyles: () => undefined,
      setInactiveStyles: () => undefined,
    })),
  );
  private readonly keys = new ActiveDescendantKeyManager(this.highlightables, inject(Injector))
    .withVerticalOrientation()
    .withHomeAndEnd()
    .withWrap();

  constructor() {
    bindValueAccessor(this);
    this.field?.register({
      showError: this.showError,
      messages: this.state.messages,
      required: this.state.required,
      labelable: true,
    });
    // The input shows the picked option's label whenever the value changes from outside.
    effect(() => {
      this.query.set(labelOf(this.options(), this.value()));
    });
    effect(() => {
      const listbox = this.listbox().nativeElement;
      const input = this.input().nativeElement;
      if (this.open()) {
        listbox.showPopover();
        listbox.style.setProperty('min-width', `${String(input.offsetWidth)}px`);
        positionPopover(input, listbox, 'below', `--${this.listboxId}`);
      } else {
        listbox.hidePopover();
      }
    });
    const changes = this.keys.change.subscribe((index) => {
      this.activeIndex.set(index);
    });
    inject(DestroyRef).onDestroy(() => {
      changes.unsubscribe();
      this.keys.destroy();
    });
  }

  protected typing(event: Event): void {
    if (event.target instanceof HTMLInputElement) {
      this.query.set(event.target.value);
      this.typed.set(true);
      this.open.set(true);
      this.activate(this.matches().length > 0 ? 0 : -1);
    }
  }

  protected keydown(event: KeyboardEvent): void {
    switch (event.key) {
      case 'Enter': {
        const active = this.matches()[this.activeIndex()];
        if (this.open() && active !== undefined) {
          event.preventDefault();
          this.choose(active.value);
        }
        return;
      }
      case 'Escape':
        if (this.open()) {
          event.preventDefault();
          this.close();
        }
        return;
      case 'ArrowDown':
      case 'ArrowUp':
        if (!this.open()) {
          event.preventDefault();
          this.show();
          return;
        }
        break;
      default:
        break;
    }
    if (this.open()) {
      this.keys.onKeydown(event);
    }
  }

  protected toggle(): void {
    if (this.open()) {
      this.close();
    } else {
      this.show();
    }
  }

  protected blurred(): void {
    this.close();
    this.onTouched();
    this.touch.emit();
  }

  /** Keeps focus in the input while an option is clicked. */
  protected keepFocus(event: MouseEvent): void {
    event.preventDefault();
  }

  /** Picks the option with `value` and closes the list. */
  choose(value: string): void {
    this.value.set(value);
    this.onChanged(value);
    this.close();
  }

  /** Closes the list; the input goes back to the picked option's label. */
  close(): void {
    this.open.set(false);
    this.typed.set(false);
    this.query.set(labelOf(this.options(), this.value()));
  }

  private show(): void {
    this.typed.set(false);
    this.open.set(true);
    const selected = this.matches().findIndex((option) => option.value === this.value());
    this.activate(selected >= 0 ? selected : 0);
  }

  private activate(index: number): void {
    if (index < 0) {
      this.activeIndex.set(-1);
      return;
    }
    this.keys.setActiveItem(index);
  }

  writeValue(value: unknown): void {
    this.value.set(typeof value === 'string' ? value : null);
  }

  registerOnChange(fn: (value: string | null) => void): void {
    this.onChanged = fn;
  }

  registerOnTouched(fn: () => void): void {
    this.onTouched = fn;
  }

  setDisabledState(disabled: boolean): void {
    this.formDisabled.set(disabled);
  }
}
