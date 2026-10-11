import {
  ChangeDetectionStrategy,
  Component,
  booleanAttribute,
  computed,
  input,
} from '@angular/core';

/**
 * One step in `<glz-breadcrumbs>`: a link to a page above, or, with `current`, the page
 * itself (`aria-current="page"`).
 *
 * ```html
 * <glz-breadcrumb><a routerLink="/">Home</a></glz-breadcrumb>
 * <glz-breadcrumb current>Settings</glz-breadcrumb>
 * ```
 */
@Component({
  selector: 'glz-breadcrumb',
  templateUrl: './breadcrumb.component.html',
  styleUrl: './breadcrumb.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
  host: {
    role: 'listitem',
    '[attr.aria-current]': 'ariaCurrent()',
  },
})
export class GlzBreadcrumbComponent {
  readonly current = input(false, { transform: booleanAttribute });

  protected readonly ariaCurrent = computed(() => (this.current() ? 'page' : null));
}
