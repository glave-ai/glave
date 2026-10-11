import { BreakpointObserver } from '@angular/cdk/layout';
import {
  ChangeDetectionStrategy,
  Component,
  computed,
  effect,
  inject,
  input,
  model,
} from '@angular/core';
import { toObservable, toSignal } from '@angular/core/rxjs-interop';
import { map, switchMap } from 'rxjs';
import { testIds } from '../../lib/test-ids';

/** The parts end-to-end tests reach, as `<testId>-<part>`. */
const PARTS = { scrim: 'scrim', nav: 'nav' } as const;

/** Below this width the side nav slides over the content instead of sitting beside it. */
export const GLZ_NARROW = '(max-width: 48rem)';

/**
 * A page with a side nav (`[glzSidenav]`) beside its content. On wide screens the nav sits
 * beside the content and `open` collapses it; on narrow screens it slides over the content
 * as a modal drawer: the content behind is inert, so focus stays in the nav, and Escape or
 * the scrim close it. Toggle it from a button with a template reference.
 *
 * ```html
 * <glz-sidenav-layout #layout label="Spaces and threads">
 *   <nav glzSidenav>…</nav>
 *   <main>…</main>
 * </glz-sidenav-layout>
 * ```
 */
@Component({
  selector: 'glz-sidenav-layout',
  templateUrl: './sidenav-layout.component.html',
  styleUrl: './sidenav-layout.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
  host: {
    '[attr.data-testid]': 'testId()',
    '[class.narrow]': 'narrow()',
    '[class.open]': 'open()',
    '(keydown.escape)': 'escape()',
  },
})
export class GlzSidenavLayoutComponent {
  /** The test hook for this component; its parts get `<testId>-<part>`. */
  readonly testId = input<string | null>(null);
  protected readonly ids = computed(() => testIds(this.testId(), PARTS));

  /** Whether the side nav shows. */
  readonly open = model(true);
  /** Names the side nav's region for screen readers. */
  readonly label = input('Navigation');
  /** The media query under which the side nav becomes a drawer. */
  readonly narrowQuery = input(GLZ_NARROW);

  private readonly breakpoints = inject(BreakpointObserver);
  /** Whether the screen is narrow, so the side nav is a drawer. */
  readonly narrow = toSignal(
    toObservable(this.narrowQuery).pipe(
      switchMap((query) => this.breakpoints.observe(query)),
      map((state) => state.matches),
    ),
    { initialValue: false },
  );
  /** Whether the side nav is open over the content, so it is modal. */
  protected readonly modal = computed(() => this.narrow() && this.open());
  protected readonly closed = computed(() => !this.open());
  protected readonly role = computed(() => (this.modal() ? 'dialog' : null));
  protected readonly ariaModal = computed(() => (this.modal() ? 'true' : null));

  constructor() {
    // A drawer starts closed whenever the screen becomes narrow; a wide screen shows the nav.
    effect(() => {
      this.open.set(!this.narrow());
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
