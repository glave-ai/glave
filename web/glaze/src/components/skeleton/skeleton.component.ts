import { ChangeDetectionStrategy, Component, computed, input } from '@angular/core';

/** A skeleton's shape: a line of text, a block, or a circle (an avatar). */
export type GlzSkeletonShape = 'text' | 'block' | 'circle';

/**
 * A placeholder in the shape of content still loading. Hidden from screen readers: pair it
 * with a `glz-spinner` or an `aria-busy` region that says what is loading.
 *
 * ```html
 * <glz-skeleton shape="text" width="60%" />
 * <glz-skeleton shape="circle" width="2rem" />
 * ```
 */
@Component({
  selector: 'glz-skeleton',
  templateUrl: './skeleton.component.html',
  styleUrl: './skeleton.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
  host: {
    'aria-hidden': 'true',
    '[class]': 'shapeClass()',
    '[style.--glz-skeleton-width]': 'width()',
    '[style.--glz-skeleton-height]': 'height()',
  },
})
export class GlzSkeletonComponent {
  readonly shape = input<GlzSkeletonShape>('text');
  /** Any CSS length. */
  readonly width = input('100%');
  /** Any CSS length; a text line's height by default. */
  readonly height = input<string | null>(null);

  protected readonly shapeClass = computed(() => `glz-skeleton-${this.shape()}`);
}
