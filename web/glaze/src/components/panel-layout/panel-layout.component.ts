import { BreakpointObserver } from '@angular/cdk/layout';
import {
  ChangeDetectionStrategy,
  Component,
  type ElementRef,
  afterRenderEffect,
  booleanAttribute,
  computed,
  inject,
  input,
  model,
  numberAttribute,
  viewChild,
} from '@angular/core';
import { toObservable, toSignal } from '@angular/core/rxjs-interop';
import { map, switchMap } from 'rxjs';

import { GlzSplitterDirective } from '../../directives/splitter.directive';
import { GlzTooltipDirective } from '../../directives/tooltip.directive';
import { testIds } from '../../lib/test-ids';
import { injectId } from '../../tokens/ids';

/** The parts end-to-end tests reach, as `<testId>-<part>`. */
const PARTS = { scrim: 'scrim', panel: 'panel', handle: 'handle' } as const;

/** Below this width the panel slides over the content as a sheet instead of sitting beside it. */
export const GLZ_PANEL_NARROW = '(max-width: 64rem)';

/**
 * A page with a panel at its end (`[glzPanel]`) beside its content: tools, files, details.
 * On wide screens the panel sits beside the content and its edge is a window splitter
 * (`glzSplitter`): drag it or use the arrow keys on it to resize, drag it past half the
 * minimum or press Enter on it to collapse, and the collapsed edge stays as a slim grip to bring
 * the panel back. `width` is in pixels, bound two ways, and `null` keeps `--glz-panel-width`.
 * On narrow screens the panel slides over the content as a modal sheet: the content behind is
 * inert, and Escape or the scrim close it. `open` is bound two ways and starts closed. Glaze
 * keeps neither: remembering them is the app's job.
 *
 * ```html
 * <glz-panel-layout label="Tools" [(open)]="toolsOpen" [(width)]="toolsWidth">
 *   <main>…</main>
 *   <section glzPanel>…</section>
 * </glz-panel-layout>
 * ```
 */
@Component({
  selector: 'glz-panel-layout',
  imports: [GlzSplitterDirective, GlzTooltipDirective],
  templateUrl: './panel-layout.component.html',
  styleUrl: './panel-layout.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
  host: {
    '[attr.data-testid]': 'testId()',
    '[class.narrow]': 'narrow()',
    '[class.open]': 'open()',
    '[style.--glz-panel-width]': 'widthStyle()',
    '(keydown.escape)': 'escape()',
  },
})
export class GlzPanelLayoutComponent {
  /** The test hook for this component; its parts get `<testId>-<part>`. */
  readonly testId = input<string | null>(null);
  protected readonly ids = computed(() => testIds(this.testId(), PARTS));

  /** Whether the panel shows. */
  readonly open = model(false);
  /** The panel's width in pixels; `null` for `--glz-panel-width`. */
  readonly width = model<number | null>(null);
  /** The narrowest the panel may be dragged while open, in pixels. */
  readonly minWidth = input(280, { transform: numberAttribute });
  /** The widest the panel may be dragged, in pixels. */
  readonly maxWidth = input(720, { transform: numberAttribute });
  /** Whether the person can resize and collapse the panel by its edge. */
  readonly resizable = input(true, { transform: booleanAttribute });
  /** Names the panel's region for screen readers. */
  readonly label = input('Panel');
  /** Names the edge. */
  readonly resizeLabel = input('Resize the panel');
  /** A hint shown beside the edge on hover and focus, such as the keys that work on it. */
  readonly resizeHint = input('');
  /** The page's shortcut that shows or hides the panel, in ARIA's form (`Meta+.`), if any. */
  readonly shortcut = input<string | null>(null);
  /** The media query under which the panel becomes a sheet over the content. */
  readonly narrowQuery = input(GLZ_PANEL_NARROW);

  protected readonly panelId = injectId('glz-panel');
  private readonly breakpoints = inject(BreakpointObserver);
  private readonly panel = viewChild.required<ElementRef<HTMLElement>>('panel');

  /** Whether the screen is narrow, so the panel is a sheet. */
  readonly narrow = toSignal(
    toObservable(this.narrowQuery).pipe(
      switchMap((query) => this.breakpoints.observe(query)),
      map((state) => state.matches),
    ),
    { initialValue: false },
  );
  /** Whether the panel is open over the content, so it is modal. */
  protected readonly modal = computed(() => this.narrow() && this.open());
  protected readonly closed = computed(() => !this.open());
  protected readonly role = computed(() => (this.modal() ? 'dialog' : 'complementary'));
  protected readonly ariaModal = computed(() => (this.modal() ? 'true' : null));
  protected readonly showHandle = computed(() => this.resizable() && !this.narrow());
  protected readonly widthStyle = computed(() => {
    const width = this.width();
    return width === null ? null : `${String(width)}px`;
  });

  constructor() {
    // A sheet takes focus as it opens, so the keyboard is where the person is looking.
    afterRenderEffect(() => {
      if (this.modal()) {
        this.panel().nativeElement.focus();
      }
    });
  }

  toggle(): void {
    this.open.update((open) => !open);
  }

  close(): void {
    this.open.set(false);
  }

  protected escape(): void {
    if (this.modal()) {
      this.close();
    }
  }
}
