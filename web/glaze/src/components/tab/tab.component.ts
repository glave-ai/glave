import {
  ChangeDetectionStrategy,
  Component,
  type TemplateRef,
  input,
  viewChild,
} from '@angular/core';

/**
 * One tab in a `<glz-tabs>`: its label, and its content, made only while it is selected.
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
