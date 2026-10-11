import {
  ChangeDetectionStrategy,
  Component,
  ElementRef,
  computed,
  effect,
  inject,
  input,
} from '@angular/core';

import { GLZ_ICONS, type GlzIconData, iconSvg } from '../../lib/icons';

/**
 * An SVG icon from Lucide (ISC licence): the icon itself, or a name registered with
 * `provideGlazeIcons`. Decorative unless given a `label`, for when the icon alone carries
 * meaning.
 *
 * ```html
 * <glz-icon [icon]="sendIcon" />
 * <glz-icon name="check" label="Done" />
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
  readonly icon = input<GlzIconData | undefined>(undefined);
  /** The name of an icon registered with `provideGlazeIcons`. */
  readonly name = input<string | undefined>(undefined);
  /** What the icon means, when nothing else says it. */
  readonly label = input<string | undefined>(undefined);

  private readonly registry = inject(GLZ_ICONS);
  private readonly host = inject<ElementRef<HTMLElement>>(ElementRef).nativeElement;

  /** The drawing: the icon given, else the one registered under `name`. */
  protected readonly drawing = computed(() => {
    const name = this.name();
    return this.icon() ?? (name === undefined ? undefined : this.registry.get(name));
  });
  protected readonly labelled = computed(() => (this.label() ?? '') !== '');
  protected readonly role = computed(() => (this.labelled() ? 'img' : null));
  protected readonly ariaLabel = computed(() => (this.labelled() ? (this.label() ?? null) : null));
  protected readonly ariaHidden = computed(() => (this.labelled() ? null : 'true'));

  constructor() {
    effect(() => {
      const drawing = this.drawing();
      this.host.replaceChildren(
        ...(drawing === undefined ? [] : [iconSvg(this.host.ownerDocument, drawing)]),
      );
    });
  }
}
