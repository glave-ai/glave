import {
  ChangeDetectionStrategy,
  Component,
  booleanAttribute,
  computed,
  inject,
  input,
} from '@angular/core';
import { GlzIconButtonComponent, GlzIconComponent, GlzPageHeaderComponent } from '@glave/glaze';
import { Menu } from 'lucide';

import { BackService } from '../../services/back.service';
import { NavService } from '../../services/nav.service';

/** Where Back goes when the person arrived at the page directly: their Chats. */
const CHATS = '/';

/**
 * The bar across the top of a page, floating over it: no border and no solid bar, but a fade
 * from the page's colour to transparent, so the page scrolls softly under it. On narrow
 * screens, where the side nav is a drawer, it holds the button that opens it; on a page that
 * is not the Chats (`back`), Back, which returns to where the person came from in glave, or
 * to their Chats when they arrived at the page directly.
 */
@Component({
  selector: 'app-page-header',
  imports: [GlzIconButtonComponent, GlzIconComponent, GlzPageHeaderComponent],
  templateUrl: './page-header.component.html',
  styleUrl: './page-header.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class PageHeaderComponent {
  /** Whether the page has a way back: every page but the Chats. */
  readonly back = input(false, { transform: booleanAttribute });

  protected readonly copy = {
    menu: $localize`:@@pageHeader.menu:Show or hide the side bar`,
    back: $localize`:@@pageHeader.back:Back`,
  } as const;
  protected readonly icon = Menu;
  private readonly nav = inject(NavService);
  private readonly backs = inject(BackService);
  protected readonly open = this.nav.open;
  protected readonly narrow = this.nav.narrow;
  protected readonly shortcut = this.nav.ariaShortcut;
  protected readonly backLabel = computed(() => (this.back() ? this.copy.back : null));

  protected toggle(): void {
    this.nav.toggle();
  }

  protected async goBack(): Promise<void> {
    await this.backs.back(CHATS);
  }
}
