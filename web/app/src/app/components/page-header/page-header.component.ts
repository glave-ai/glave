import { ChangeDetectionStrategy, Component, inject } from '@angular/core';
import { GlzIconButtonComponent, GlzIconComponent, GlzTopBarComponent } from '@glave/glaze';
import { PanelLeft } from 'lucide';

import { NavService } from '../../services/nav.service';

const COPY = {
  menu: 'Show or hide the side bar',
} as const;

/**
 * The bar across the top of a page: the side nav's toggle, the page's own heading (projected),
 * and its actions at the end (`[appPageHeaderEnd]`).
 */
@Component({
  selector: 'app-page-header',
  imports: [GlzIconButtonComponent, GlzIconComponent, GlzTopBarComponent],
  templateUrl: './page-header.component.html',
  styleUrl: './page-header.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class PageHeaderComponent {
  protected readonly copy = COPY;
  protected readonly icon = PanelLeft;
  private readonly nav = inject(NavService);
  protected readonly open = this.nav.open;

  protected toggle(): void {
    this.nav.toggle();
  }
}
