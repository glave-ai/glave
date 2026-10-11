import { CdkMenu } from '@angular/cdk/menu';
import { ChangeDetectionStrategy, Component } from '@angular/core';

/**
 * A menu of actions, opened by a `glzMenuTrigger`: the Angular CDK's menu, so arrow keys,
 * typeahead, Escape and focus return all work, in Glaze's look. Put it in an
 * `<ng-template>` the trigger points at; its items are `glzMenuItem` buttons.
 *
 * ```html
 * <button glzIconButton label="Thread actions" [glzMenuTrigger]="actions"><glz-icon name="more" /></button>
 * <ng-template #actions>
 *   <glz-menu>
 *     <button glzMenuItem (triggered)="rename()">Rename</button>
 *     <button glzMenuItem (triggered)="archive()">Archive</button>
 *   </glz-menu>
 * </ng-template>
 * ```
 */
@Component({
  selector: 'glz-menu',
  hostDirectives: [{ directive: CdkMenu, outputs: ['closed'] }],
  templateUrl: './menu.component.html',
  styleUrl: './menu.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class GlzMenuComponent {}
