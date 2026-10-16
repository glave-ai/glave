import { ChangeDetectionStrategy, Component, inject } from '@angular/core';
import { GlzIconButtonComponent, GlzIconComponent, GlzTopBarComponent } from '@glave/glaze';
import { Menu } from 'lucide';

import { NavService } from '../../services/nav.service';

const COPY = {
  menu: 'Show or hide the side bar',
} as const;

/**
 * The bar across the top of a page, floating over it: no border and no solid bar, but a fade
 * from the page's colour to transparent, so the page scrolls softly under it. It holds the
 * page's own heading (projected), its actions at the end (`[appPageHeaderEnd]`) and, on narrow
 * screens, where the side nav is a drawer, the button that opens it.
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
  protected readonly icon = Menu;
  private readonly nav = inject(NavService);
  protected readonly open = this.nav.open;
  protected readonly narrow = this.nav.narrow;
  protected readonly shortcut = this.nav.ariaShortcut;

  protected toggle(): void {
    this.nav.toggle();
  }
}
