import {
  ChangeDetectionStrategy,
  Component,
  type TemplateRef,
  input,
  viewChild,
} from '@angular/core';

/**
 * One tab in a `<glz-tabs>`: its label and its content.
 *
 * The content is made with the tab, selected or not, and lives as long as the tab does:
 * `<glz-tabs>` only puts the selected tab's content on the page and takes it off again. Content
 * that should do nothing until its tab is chosen (start a shell, open a connection) goes
 * behind an `@if` in the template that holds the tab.
 *
 * ```html
 * <glz-tab label="Events">…</glz-tab>
 * ```
 */
@Component({
  selector: 'glz-tab',
  templateUrl: './tab.component.html',
  styleUrl: './tab.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class GlzTabComponent {
  readonly label = input.required<string>();

  /** The tab's content, which `<glz-tabs>` shows in its panel. */
  readonly content = viewChild.required<TemplateRef<unknown>>('content');
}
