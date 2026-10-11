import { ChangeDetectionStrategy, Component, input } from '@angular/core';

/**
 * Where this page sits: a trail of `<glz-breadcrumb>`s from the top down, the last one the
 * page itself. A landmark named by `label`.
 *
 * ```html
 * <glz-breadcrumbs>
 *   <glz-breadcrumb><a routerLink="/spaces/glave">glave</a></glz-breadcrumb>
 *   <glz-breadcrumb current>Plan the launch</glz-breadcrumb>
 * </glz-breadcrumbs>
 * ```
 */
@Component({
  selector: 'glz-breadcrumbs',
  templateUrl: './breadcrumbs.component.html',
  styleUrl: './breadcrumbs.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class GlzBreadcrumbsComponent {
  readonly label = input('Breadcrumb');
}
