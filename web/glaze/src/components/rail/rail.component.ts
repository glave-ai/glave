import { ChangeDetectionStrategy, Component } from '@angular/core';

/**
 * A narrow column of top-level destinations, each a `glzRailItem` with an icon and a short
 * label. Put it in a `<nav>` named for what it lists.
 *
 * ```html
 * <nav aria-label="Main">
 *   <glz-rail>
 *     <a glzRailItem routerLink="/chat" [icon]="chatIcon" label="Chat" [current]="true"></a>
 *     <a glzRailItem routerLink="/memory" [icon]="memoryIcon" label="Memory"></a>
 *   </glz-rail>
 * </nav>
 * ```
 */
@Component({
  selector: 'glz-rail',
  templateUrl: './rail.component.html',
  styleUrl: './rail.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class GlzRailComponent {}
