import {
  ChangeDetectionStrategy,
  Component,
  type ElementRef,
  afterRenderEffect,
  computed,
  effect,
  input,
  model,
  untracked,
  viewChild,
} from '@angular/core';

import { GlzSplitterDirective } from '../../directives/splitter.directive';
import { GlzTooltipDirective } from '../../directives/tooltip.directive';
import { mediaMatches } from '../../lib/media';
import { testIds } from '../../lib/test-ids';
import { injectId } from '../../tokens/ids';
import { glzText } from '../../lib/words';

/** The parts end-to-end tests reach, as `<testId>-<part>`. */
const PARTS = { scrim: 'scrim', nav: 'nav', handle: 'handle' } as const;

/** Below this width the side nav slides over the content instead of sitting beside it. */
export const GLZ_NARROW = '(max-width: 48rem)';

/**
 * A page with a side nav (`[glzSidenav]`) beside its content. On wide screens the nav sits
 * beside the content and its edge is a window splitter (`glzSplitter`): drag it or use the
 * arrow keys on it to resize, drag it past half the minimum or press Enter on it to collapse,
 * and the collapsed edge stays as a slim grip to bring the nav back. `open` and `width` (pixels,
 * `null` for `--glz-sidenav-width`) bind two ways. On narrow screens the nav slides over the
 * content as a modal drawer: it takes focus as it opens, the content behind is inert, so focus
 * stays in the nav, and Escape or the scrim close it. Toggle it from a button with a template
 * reference. The drawer starts closed; going back to a wide screen brings back the nav as it was.
 *
 * ```html
 * <glz-sidenav-layout #layout label="Spaces and threads" [(open)]="open" [(width)]="width">
 *   <nav glzSidenav>…</nav>
 *   <main>…</main>
 * </glz-sidenav-layout>
 * ```
 */
@Component({
  selector: 'glz-sidenav-layout',
  imports: [GlzSplitterDirective, GlzTooltipDirective],
  templateUrl: './sidenav-layout.component.html',
  styleUrl: './sidenav-layout.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
  host: {
    '[attr.data-testid]': 'testId()',
    '[class.narrow]': 'narrow()',
    // Closed is the class, so before its first binding (a route's page is added a frame before
    // Angular binds it) it is drawn open, as it starts, and nothing moves as it is bound.
    '[class.closed]': 'closed()',
    '[style.--glz-sidenav-width]': 'widthStyle()',
    '(keydown.escape)': 'escape()',
  },
})
export class GlzSidenavLayoutComponent {
  /** The test hook for this component; its parts get `<testId>-<part>`. */
  readonly testId = input<string | null>(null);
  protected readonly ids = computed(() => testIds(this.testId(), PARTS));
  private readonly words = glzText('sidenavLayout', {
    label: 'Navigation',
    resizeLabel: 'Resize the navigation',
  });

  /** Whether the side nav shows. */
  readonly open = model(true);
  /** The side nav's width in pixels; `null` for `--glz-sidenav-width`. */
  readonly width = model<number | null>(null);
  /** Names the side nav's region for screen readers. */
  readonly label = input(this.words.label);
  /** Names the edge. */
  readonly resizeLabel = input(this.words.resizeLabel);
  /** A hint shown beside the edge on hover and focus, such as the keys that work on it. */
  readonly resizeHint = input('');
  /** The page's shortcut that shows or hides the side nav, in ARIA's form (`Meta+B`), if any. */
  readonly shortcut = input<string | null>(null);
  /** The media query under which the side nav becomes a drawer. */
  readonly narrowQuery = input(GLZ_NARROW);

  protected readonly navId = injectId('glz-sidenav');
  private readonly sidenav = viewChild.required<ElementRef<HTMLElement>>('sidenav');
  /** Whether the screen is narrow, so the side nav is a drawer. */
  readonly narrow = mediaMatches(this.narrowQuery);
  /** Whether the side nav is open over the content, so it is modal. */
  protected readonly modal = computed(() => this.narrow() && this.open());
  protected readonly closed = computed(() => !this.open());
  protected readonly role = computed(() => (this.modal() ? 'dialog' : null));
  protected readonly ariaModal = computed(() => (this.modal() ? 'true' : null));
  protected readonly showHandle = computed(() => !this.narrow());
  /** The narrowest the side nav may be dragged while open, and the widest, in pixels. */
  protected readonly minWidth = 200;
  protected readonly maxWidth = 480;
  protected readonly widthStyle = computed(() => {
    const width = this.width();
    return width === null ? null : `${String(width)}px`;
  });

  constructor() {
    // The drawer starts closed whenever the screen becomes narrow; back on a wide screen the
    // nav is as the person left it there.
    let wideOpen: boolean | null = null;
    effect(() => {
      const narrow = this.narrow();
      untracked(() => {
        if (narrow && wideOpen === null) {
          wideOpen = this.open();
          this.open.set(false);
        } else if (!narrow && wideOpen !== null) {
          this.open.set(wideOpen);
          wideOpen = null;
        }
      });
    });
    // A drawer takes focus as it opens: the button that opened it is now behind it, inert.
    afterRenderEffect(() => {
      if (this.modal()) {
        this.sidenav().nativeElement.focus();
      }
    });
  }

  toggle(): void {
    this.open.update((open) => !open);
  }

  close(): void {
    this.open.set(false);
  }

  /** Closes the nav after following a link in it, when it is a drawer. */
  navigated(): void {
    if (this.narrow()) {
      this.close();
    }
  }

  protected escape(): void {
    if (this.modal()) {
      this.close();
    }
  }
}
