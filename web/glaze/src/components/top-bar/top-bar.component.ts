import { ChangeDetectionStrategy, Component } from '@angular/core';

/**
 * The bar across the top of the app: things at the start (`[glzTopBarStart]`, such as the
 * button that opens the side nav), the title in the middle, actions at the end
 * (`[glzTopBarEnd]`). Put it in the page's `<header>`.
 *
 * ```html
 * <header>
 *   <glz-top-bar>
 *     <button glzTopBarStart glzIconButton label="Menu" (click)="layout.toggle()"><glz-icon [icon]="menuIcon" /></button>
 *     <span>Atlas</span>
 *     <glz-theme-toggle glzTopBarEnd />
 *   </glz-top-bar>
 * </header>
 * ```
 */
@Component({
  selector: 'glz-top-bar',
  templateUrl: './top-bar.component.html',
  styleUrl: './top-bar.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class GlzTopBarComponent {}
