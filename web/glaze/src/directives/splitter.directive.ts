import { Directionality } from '@angular/cdk/bidi';
import {
  Directive,
  afterRenderEffect,
  computed,
  inject,
  input,
  model,
  numberAttribute,
  signal,
} from '@angular/core';

import { type GlzPanelSide, clampWidth, draggedSplit, splitterKeys } from '../lib/resize';

/** How far one arrow key moves the edge, in pixels. */
const STEP = 16;

/** A pointer that moves less than this, in pixels, clicked rather than dragged. */
const CLICK_SLOP = 3;

/** A drag under way: the pointer, where it started, and the panel's width then (0 if collapsed). */
interface Drag {
  readonly pointer: number;
  readonly startX: number;
  readonly startWidth: number;
  readonly moved: boolean;
}

/**
 * The edge of a resizable panel: the WAI-ARIA window splitter. A focusable `separator` that
 * controls the panel (`glzSplitter` is the panel's id, `glzSplitterPanel` the element) and
 * says its width as its value (0 when collapsed). Dragging resizes the panel; dragging it below
 * half its minimum collapses it, and dragging or clicking the collapsed edge brings it back.
 * The arrow keys move the edge, Home and End take it to the bounds, and Enter collapses the
 * panel or restores it. `open` and `width` (pixels, `null` for the panel's own) bind two ways.
 * Glaze's layouts use it; it carries no look of its own.
 *
 * ```html
 * <div glzSplitter="files" [glzSplitterPanel]="files" side="end" [(open)]="open" [(width)]="width"
 *   aria-label="Resize the files"></div>
 * <aside #files id="files">…</aside>
 * ```
 */
@Directive({
  selector: '[glzSplitter]',
  host: {
    role: 'separator',
    tabindex: '0',
    'aria-orientation': 'vertical',
    'aria-valuemin': '0',
    '[class.dragging]': 'dragging()',
    '[attr.aria-controls]': 'controls()',
    '[attr.aria-valuenow]': 'valueNow()',
    '[attr.aria-valuemax]': 'maxWidth()',
    '(pointerdown)': 'grab($event)',
    '(pointermove)': 'moved($event)',
    '(pointerup)': 'released($event)',
    '(pointercancel)': 'cancelled($event)',
    '(keydown)': 'keydown($event)',
  },
})
export class GlzSplitterDirective {
  /** The id of the panel this edge resizes. */
  readonly controls = input.required<string>({ alias: 'glzSplitter' });
  /** The panel, measured while its width is still its own. */
  readonly panel = input.required<HTMLElement>({ alias: 'glzSplitterPanel' });
  /** The side of the page the panel sits on. */
  readonly side = input<GlzPanelSide>('end');
  /** The narrowest the panel may be while open, in pixels. */
  readonly minWidth = input(280, { transform: numberAttribute });
  /** The widest the panel may be, in pixels. */
  readonly maxWidth = input(720, { transform: numberAttribute });
  /** Whether the panel is open; `false` is collapsed. */
  readonly open = model(true);
  /** The panel's width in pixels, kept while it is collapsed; `null` for its own. */
  readonly width = model<number | null>(null);

  private readonly direction = inject(Directionality);
  private readonly drag = signal<Drag | null>(null);
  /** The panel's own width, measured while `width` is `null`. */
  private readonly measured = signal<number | null>(null);
  protected readonly dragging = computed(() => this.drag() !== null);
  protected readonly valueNow = computed(() =>
    this.open() ? (this.width() ?? this.measured() ?? this.minWidth()) : 0,
  );

  constructor() {
    afterRenderEffect(() => {
      if (this.open() && this.width() === null) {
        const width = this.panel().getBoundingClientRect().width;
        this.measured.set(width > 0 ? Math.round(width) : null);
      }
    });
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
      startWidth: this.open() ? this.currentWidth() : 0,
      moved: false,
    });
  }

  protected moved(event: PointerEvent): void {
    const drag = this.drag();
    if (drag?.pointer !== event.pointerId) {
      return;
    }
    const delta = event.clientX - drag.startX;
    if (!drag.moved && Math.abs(delta) < CLICK_SLOP) {
      return;
    }
    this.drag.set({ ...drag, moved: true });
    const split = draggedSplit(
      drag.startWidth,
      delta,
      this.side(),
      this.direction.value,
      this.minWidth(),
      this.maxWidth(),
    );
    this.open.set(split.open);
    if (split.open) {
      this.width.set(split.width);
    }
  }

  protected released(event: PointerEvent): void {
    const drag = this.drag();
    if (drag?.pointer !== event.pointerId) {
      return;
    }
    this.drag.set(null);
    // A click on the collapsed edge brings the panel back.
    if (!drag.moved && drag.startWidth === 0) {
      this.open.set(true);
    }
  }

  protected cancelled(event: PointerEvent): void {
    if (this.drag()?.pointer === event.pointerId) {
      this.drag.set(null);
    }
  }

  protected keydown(event: KeyboardEvent): void {
    const { grow, shrink } = splitterKeys(this.side(), this.direction.value);
    const min = this.minWidth();
    const max = this.maxWidth();
    if (event.key === 'Enter') {
      event.preventDefault();
      this.open.update((open) => !open);
      return;
    }
    if (!this.open()) {
      // Collapsed, the edge only opens: the arrow that widens, or End.
      if (event.key === grow || event.key === 'End') {
        event.preventDefault();
        this.open.set(true);
      }
      return;
    }
    const now = this.currentWidth();
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
    return this.width() ?? this.panel().getBoundingClientRect().width;
  }
}
