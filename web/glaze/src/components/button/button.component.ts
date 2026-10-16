import {
  ChangeDetectionStrategy,
  Component,
  booleanAttribute,
  computed,
  input,
} from '@angular/core';

/** How much a button stands out: one primary action per view, the rest quieter. */
export type GlzButtonVariant = 'primary' | 'secondary' | 'ghost' | 'danger';

/** A button's size. */
export type GlzButtonSize = 'sm' | 'md' | 'lg';

/** The classes for a button of `variant` and `size`. */
export function buttonClasses(variant: GlzButtonVariant, size: GlzButtonSize): string {
  return `glz-button glz-button--${variant} glz-button--${size}`;
}

/**
 * A button, or a link that looks like one. The host stays the native element, so the
 * browser's keyboard, form and screen-reader behaviour all still apply. A `pill` one is fully
 * rounded, for a button that sits in a bar beside text, such as a page's "Changes".
 *
 * ```html
 * <button glzButton variant="primary" (click)="save()">Save</button>
 * <a glzButton href="/help">Help</a>
 * <button glzButton size="sm" pill (click)="show()">Changes</button>
 * ```
 */
@Component({
  selector: 'button[glzButton], a[glzButton]',
  templateUrl: './button.component.html',
  styleUrl: './button.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
  host: { '[class]': 'classes()', '[class.glz-button--pill]': 'pill()' },
})
export class GlzButtonComponent {
  readonly variant = input<GlzButtonVariant>('secondary');
  readonly size = input<GlzButtonSize>('md');
  /** Whether it is fully rounded. */
  readonly pill = input(false, { transform: booleanAttribute });

  protected readonly classes = computed(() => buttonClasses(this.variant(), this.size()));
}
