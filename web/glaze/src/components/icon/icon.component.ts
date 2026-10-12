import {
  ChangeDetectionStrategy,
  Component,
  ElementRef,
  computed,
  effect,
  inject,
  input,
} from '@angular/core';

import { type GlzIconData, iconSvg } from '../../lib/icons';

/**
 * An SVG icon from Lucide (ISC licence). Only the icons imported are bundled: Lucide ships each
 * as its own ES module. Decorative unless given a `label`, for when the icon alone carries
 * meaning.
 *
 * ```html
 * <glz-icon [icon]="sendIcon" />
 * <glz-icon [icon]="checkIcon" label="Done" />
 * ```
 */
@Component({
  selector: 'glz-icon',
  templateUrl: './icon.component.html',
  styleUrl: './icon.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
  host: {
    '[attr.role]': 'role()',
    '[attr.aria-label]': 'ariaLabel()',
    '[attr.aria-hidden]': 'ariaHidden()',
  },
})
export class GlzIconComponent {
  /** The icon to draw, such as `Send` from `lucide`. */
  readonly icon = input.required<GlzIconData>();
  /** What the icon means, when nothing else says it. */
  readonly label = input<string | undefined>(undefined);

  private readonly host = inject<ElementRef<HTMLElement>>(ElementRef).nativeElement;

  protected readonly labelled = computed(() => (this.label() ?? '') !== '');
  protected readonly role = computed(() => (this.labelled() ? 'img' : null));
  protected readonly ariaLabel = computed(() => (this.labelled() ? (this.label() ?? null) : null));
  protected readonly ariaHidden = computed(() => (this.labelled() ? null : 'true'));

  constructor() {
    effect(() => {
      this.host.replaceChildren(iconSvg(this.host.ownerDocument, this.icon()));
    });
  }
}
