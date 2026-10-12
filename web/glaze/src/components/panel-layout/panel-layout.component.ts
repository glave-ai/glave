import { Directionality } from '@angular/cdk/bidi';
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
  signal,
  viewChild,
} from '@angular/core';
import { toObservable, toSignal } from '@angular/core/rxjs-interop';
import { map, switchMap } from 'rxjs';

import { clampWidth, resizedWidth } from '../../lib/resize';
import { testIds } from '../../lib/test-ids';
import { injectId } from '../../tokens/ids';

/** The parts end-to-end tests reach, as `<testId>-<part>`. */
const PARTS = { scrim: 'scrim', panel: 'panel', handle: 'handle' } as const;

/** Below this width the panel slides over the content as a sheet instead of sitting beside it. */
export const GLZ_PANEL_NARROW = '(max-width: 64rem)';

/** How far one arrow key moves the panel's edge, in pixels. */
const STEP = 16;

/** A drag under way: where the pointer and the panel's width started. */
interface Drag {
  readonly pointer: number;
  readonly startX: number;
  readonly startWidth: number;
}

/**
 * A page with a panel at its end (`[glzPanel]`) beside its content: tools, files, details.
 * On wide screens the panel sits beside the content, and the person resizes it by dragging
 * its edge or with the arrow keys on it (a focusable `separator`, the ARIA window splitter);
 * `width` is in pixels, bound two ways, and `null` keeps `--glz-panel-width`. On narrow
 * screens the panel slides over the content as a modal sheet: the content behind is inert,
 * and Escape or the scrim close it. `open` is bound two ways and starts closed. Glaze keeps
 * neither: remembering them is the app's job.
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
  /** The narrowest the panel may be dragged, in pixels. */
  readonly minWidth = input(280, { transform: numberAttribute });
  /** The widest the panel may be dragged, in pixels. */
  readonly maxWidth = input(720, { transform: numberAttribute });
  /** Whether the person can resize the panel. */
  readonly resizable = input(true, { transform: booleanAttribute });
  /** Names the panel's region for screen readers. */
  readonly label = input('Panel');
  /** Names the resize handle. */
  readonly resizeLabel = input('Resize the panel');
  /** The media query under which the panel becomes a sheet over the content. */
  readonly narrowQuery = input(GLZ_PANEL_NARROW);

  protected readonly panelId = injectId('glz-panel');
  private readonly breakpoints = inject(BreakpointObserver);
  private readonly direction = inject(Directionality);
  private readonly panel = viewChild.required<ElementRef<HTMLElement>>('panel');
  private readonly drag = signal<Drag | null>(null);

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
  protected readonly dragging = computed(() => this.drag() !== null);
  protected readonly widthStyle = computed(() => {
    const width = this.width();
    return width === null ? null : `${String(width)}px`;
  });
  /** What the separator says: the width, as ARIA's value. */
  protected readonly valueNow = computed(() => this.width() ?? this.minWidth());

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

  protected grab(event: PointerEvent): void {
    if (event.button !== 0 || !(event.target instanceof HTMLElement)) {
      return;
    }
    event.preventDefault();
    event.target.setPointerCapture(event.pointerId);
    this.drag.set({
      pointer: event.pointerId,
      startX: event.clientX,
      startWidth: this.currentWidth(),
    });
  }

  protected moved(event: PointerEvent): void {
    const drag = this.drag();
    if (drag?.pointer !== event.pointerId) {
      return;
    }
    const delta = event.clientX - drag.startX;
    this.width.set(
      resizedWidth(drag.startWidth, delta, this.direction.value, this.minWidth(), this.maxWidth()),
    );
  }

  protected released(event: PointerEvent): void {
    if (this.drag()?.pointer === event.pointerId) {
      this.drag.set(null);
    }
  }

  protected keydown(event: KeyboardEvent): void {
    const min = this.minWidth();
    const max = this.maxWidth();
    const now = this.currentWidth();
    // The panel is at the end: moving its edge toward the start makes it wider.
    const grow = this.direction.value === 'rtl' ? 'ArrowRight' : 'ArrowLeft';
    const shrink = this.direction.value === 'rtl' ? 'ArrowLeft' : 'ArrowRight';
    const next = new Map<string, number>([
      [grow, now + STEP],
      [shrink, now - STEP],
      ['Home', min],
      ['End', max],
    ]).get(event.key);
    if (next !== undefined) {
      event.preventDefault();
      this.width.set(clampWidth(next, min, max));
    }
  }

  private currentWidth(): number {
    return this.width() ?? this.panel().nativeElement.getBoundingClientRect().width;
  }
}
