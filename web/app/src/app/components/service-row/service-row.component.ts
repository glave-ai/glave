import { ChangeDetectionStrategy, Component, computed, input } from '@angular/core';
import { initials } from '@glave/glaze';

/**
 * A service in a list of them: a tile with its letters until it has a logo, its name, a line
 * about it under that (projected), and what can be done with it at the end of the row (an
 * element marked `appServiceAction`). Its host is the list's item.
 */
@Component({
  selector: 'app-service-row',
  templateUrl: './service-row.component.html',
  styleUrl: './service-row.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
  host: { role: 'listitem' },
})
export class ServiceRowComponent {
  /** What the person calls the service. */
  readonly name = input.required<string>();

  protected readonly letters = computed(() => initials(this.name()));
}
