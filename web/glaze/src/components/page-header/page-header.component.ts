import { ChangeDetectionStrategy, Component, computed, input, output } from '@angular/core';
import { ArrowLeft } from 'lucide';

import { testIds } from '../../lib/test-ids';
import { GlzButtonComponent } from '../button/button.component';
import { GlzIconComponent } from '../icon/icon.component';
import { GlzTopBarComponent } from '../top-bar/top-bar.component';

/** The parts end-to-end tests reach, as `<testId>-<part>`. */
const PARTS = { back: 'back' } as const;

/**
 * The header of a page that is not the app's main one (its settings, a list): a top bar with,
 * at its start, what the app puts there (`[glzPageHeaderStart]`, such as the button that opens
 * a side nav) and then, given a `backLabel`, a Back control, an arrow and the label, that
 * emits `back`. Where Back goes is the app's to decide. The arrow turns round in right-to-left
 * text.
 *
 * ```html
 * <glz-page-header testId="page" backLabel="Back" (back)="goBack()" />
 * ```
 */
@Component({
  selector: 'glz-page-header',
  imports: [GlzButtonComponent, GlzIconComponent, GlzTopBarComponent],
  templateUrl: './page-header.component.html',
  styleUrl: './page-header.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class GlzPageHeaderComponent {
  /** The test hook for this component; its parts get `<testId>-<part>`. */
  readonly testId = input<string | null>(null);
  protected readonly ids = computed(() => testIds(this.testId(), PARTS));

  /** The Back control's words; with none, there is no Back control. */
  readonly backLabel = input<string | null>(null);
  readonly back = output();

  protected readonly arrow = ArrowLeft;

  protected wentBack(): void {
    this.back.emit();
  }
}
