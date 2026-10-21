import { ChangeDetectionStrategy, Component, input, output } from '@angular/core';
import { GlzMenuButtonComponent, type GlzOption } from '@glave/glaze';

/**
 * A list's ⋯ in the side nav: what can be done with it (a project's Rename). Its own file, so
 * the side nav can load it, and the CDK's menu with it, once the page is idle rather than in
 * the app's first load.
 */
@Component({
  selector: 'app-nav-list-menu',
  imports: [GlzMenuButtonComponent],
  templateUrl: './nav-list-menu.component.html',
  styleUrl: './nav-list-menu.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class NavListMenuComponent {
  /** The test hook of its menu button; its parts get `<testId>-<part>`. */
  readonly testId = input<string | null>(null);
  /** What the menu is for, such as "Options for glave". */
  readonly label = input.required<string>();
  /** What can be done with the list. */
  readonly actions = input.required<readonly GlzOption[]>();
  /** Emitted with an action's value when the person picks it. */
  readonly action = output<string>();

  protected act(value: string): void {
    this.action.emit(value);
  }
}
