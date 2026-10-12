import {
  ChangeDetectionStrategy,
  Component,
  booleanAttribute,
  computed,
  input,
} from '@angular/core';

import {
  type GlzButtonSize,
  type GlzButtonVariant,
  buttonClasses,
} from '../button/button.component';

/**
 * A button that shows only an icon. Its `label` is required: it names the button for screen
 * readers and, unless `tooltip` is off, is shown on hover.
 *
 * ```html
 * <button glzIconButton label="Close" (click)="close()"><glz-icon [icon]="closeIcon" /></button>
 * ```
 */
@Component({
  selector: 'button[glzIconButton], a[glzIconButton]',
  templateUrl: './icon-button.component.html',
  styleUrl: './icon-button.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
  host: {
    '[class]': 'classes()',
    '[attr.aria-label]': 'label()',
    '[attr.title]': 'title()',
  },
})
export class GlzIconButtonComponent {
  /** What the button does, in words. */
  readonly label = input.required<string>();
  readonly variant = input<GlzButtonVariant>('ghost');
  readonly size = input<GlzButtonSize>('md');
  /** Whether hovering shows the label; turn it off when a `glzTooltip` shows it instead. */
  readonly tooltip = input(true, { transform: booleanAttribute });

  protected readonly classes = computed(
    () => `${buttonClasses(this.variant(), this.size())} glz-icon-button`,
  );
  protected readonly title = computed(() => (this.tooltip() ? this.label() : null));
}
