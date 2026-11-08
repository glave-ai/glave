import {
  CSP_NONCE,
  ChangeDetectionStrategy,
  Component,
  DOCUMENT,
  DestroyRef,
  type ElementRef,
  InjectionToken,
  ViewEncapsulation,
  afterNextRender,
  inject,
  input,
  viewChild,
} from '@angular/core';
import type { TerminalId } from '@glave/client';
import type { FitAddon } from '@xterm/addon-fit';
import type { ILocalizableStrings, ITerminalOptions, Terminal } from '@xterm/xterm';

import { type Size, TerminalsService } from '../../services/terminals.service';

/** What draws a terminal: xterm.js, the few words it says itself, and its fit addon. */
export interface Emulator {
  readonly Terminal: new (options: ITerminalOptions) => Terminal;
  readonly strings: ILocalizableStrings;
  readonly FitAddon: new () => FitAddon;
}

/**
 * Loads the emulator the first time a terminal shows, so the app's first load carries none of it.
 */
export const TERMINAL_EMULATOR = new InjectionToken<() => Promise<Emulator>>('TERMINAL_EMULATOR', {
  providedIn: 'root',
  factory: () => async () => {
    const [{ Terminal }, { FitAddon }] = await Promise.all([
      import('@xterm/xterm'),
      import('@xterm/addon-fit'),
    ]);
    return { Terminal, strings: Terminal.strings, FitAddon };
  },
});

/** The sixteen colours programs print in, as xterm.js names them, in their order. */
const ANSI = [
  'black',
  'red',
  'green',
  'yellow',
  'blue',
  'magenta',
  'cyan',
  'white',
  'brightBlack',
  'brightRed',
  'brightGreen',
  'brightYellow',
  'brightBlue',
  'brightMagenta',
  'brightCyan',
  'brightWhite',
] as const;

/** The size of a terminal's type where its screen says none. */
const FONT_SIZE = 13;

/** How many lines a terminal keeps above what it shows. */
const SCROLLBACK = 5_000;

/**
 * Runs `open` with each `<style>` the page makes meanwhile carrying `nonce`: xterm.js adds styles
 * of its own as it opens, which the page's Content Security Policy admits only so.
 */
function nonced(document: Document, nonce: string | null, open: () => void): void {
  // The page's own, from its prototype, called as the page calls it.
  const make: unknown = Reflect.get(Document.prototype, 'createElement');
  if (nonce === null || typeof make !== 'function') {
    open();
    return;
  }
  Object.defineProperty(document, 'createElement', {
    configurable: true,
    value: (...args: readonly unknown[]): unknown => {
      const made: unknown = Reflect.apply(make, document, args);
      if (made instanceof HTMLStyleElement) {
        made.nonce = nonce;
      }
      return made;
    },
  });
  try {
    open();
  } finally {
    Reflect.deleteProperty(document, 'createElement');
  }
}

/** Whether two sizes are the same. */
function same(a: Size | null, b: Size): boolean {
  return a !== null && a.cols === b.cols && a.rows === b.rows;
}

/**
 * One terminal's screen, drawn by xterm.js: what its shell prints, from its start, and what is
 * typed into it, sent in order; it fills its box, and its shell is told each size it takes. Dark
 * in both themes, as terminals are, in the mono face. Its own styles reach what xterm draws.
 */
@Component({
  selector: 'app-terminal-screen',
  templateUrl: './terminal-screen.component.html',
  styleUrl: './terminal-screen.component.scss',
  encapsulation: ViewEncapsulation.None,
  changeDetection: ChangeDetectionStrategy.OnPush,
  host: { class: 'app-terminal-screen' },
})
export class TerminalScreenComponent {
  /** The terminal it shows. */
  readonly terminal = input.required<TerminalId>();
  private readonly service = inject(TerminalsService);
  private readonly screen = viewChild.required<ElementRef<HTMLElement>>('screen');
  private readonly destroyed = inject(DestroyRef);
  private readonly stop = new AbortController();
  private drawn: Terminal | null = null;

  constructor() {
    const load = inject(TERMINAL_EMULATOR);
    const nonce = inject(CSP_NONCE, { optional: true });
    const document = inject(DOCUMENT);
    this.destroyed.onDestroy(() => {
      this.stop.abort();
    });
    afterNextRender(() => {
      void this.draw(load, nonce, document);
    });
  }

  /** Puts the keyboard's place in it. */
  focus(): void {
    this.drawn?.focus();
  }

  /**
   * Loads the emulator, then draws once the screen is on the page with room to draw in, which
   * may be a moment after it renders, as its tab is put in place; where there is no
   * ResizeObserver (a test's page), at once.
   */
  private async draw(
    load: () => Promise<Emulator>,
    nonce: string | null,
    document: Document,
  ): Promise<void> {
    const emulator = await load();
    if (this.stop.signal.aborted) {
      return;
    }
    const element = this.screen().nativeElement;
    const view = document.defaultView;
    if (typeof view?.ResizeObserver !== 'function') {
      this.open(emulator, nonce, element);
      return;
    }
    let fitted: (() => void) | null = null;
    const watching = new view.ResizeObserver(() => {
      if (element.isConnected && element.clientWidth > 0) {
        fitted ??= this.open(emulator, nonce, element);
        fitted();
      }
    });
    watching.observe(element);
    this.destroyed.onDestroy(() => {
      watching.disconnect();
    });
  }

  /**
   * Draws the terminal in `element`, on the page, in Glaze's tokens, follows what its shell
   * prints and sends what is typed; answers what fits it to its box again.
   */
  private open(emulator: Emulator, nonce: string | null, element: HTMLElement): () => void {
    const document = element.ownerDocument;
    // Glaze's tokens are the page's own, on its root; the size of its type, the screen's.
    const tokens = getComputedStyle(document.documentElement);
    const color = (name: string): string => tokens.getPropertyValue(name).trim();
    // Its own words, which screen readers hear, in the person's language.
    emulator.strings.promptLabel = $localize`:@@terminalScreen.prompt:Terminal input`;
    emulator.strings.tooMuchOutput = $localize`:@@terminalScreen.tooMuch:Too much printed to read aloud`;
    const drawn = new emulator.Terminal({
      screenReaderMode: true,
      fontFamily: color('--glz-font-mono'),
      fontSize: Number.parseFloat(getComputedStyle(element).fontSize) || FONT_SIZE,
      scrollback: SCROLLBACK,
      theme: {
        background: color('--glz-surface-terminal'),
        foreground: color('--glz-color-on-terminal'),
        cursor: color('--glz-color-on-terminal'),
        selectionBackground: color('--glz-terminal-selection'),
        ...Object.fromEntries(
          ANSI.map((name, index) => [name, color(`--glz-terminal-ansi-${String(index)}`)]),
        ),
      },
    });
    const fit = new emulator.FitAddon();
    drawn.loadAddon(fit);
    nonced(document, nonce, () => {
      drawn.open(element);
    });
    this.drawn = drawn;
    const id = this.terminal();
    const encoder = new TextEncoder();
    const typed = drawn.onData((data) => {
      this.service.type(id, encoder.encode(data));
    });
    // What xterm.js sends as bytes, not text (a mouse report a program asked for): each
    // character one byte.
    const sent = drawn.onBinary((data) => {
      this.service.type(
        id,
        Uint8Array.from(data, (character) => character.charCodeAt(0) % 256),
      );
    });
    this.destroyed.onDestroy(() => {
      typed.dispose();
      sent.dispose();
      drawn.dispose();
    });
    let size: Size | null = null;
    const fitted = (): void => {
      fit.fit();
      const now = { cols: drawn.cols, rows: drawn.rows };
      if (!same(size, now)) {
        size = now;
        void this.service.resize(id, now);
      }
    };
    fitted();
    void this.service.follow(
      id,
      (bytes) => {
        drawn.write(bytes);
      },
      this.stop.signal,
    );
    return fitted;
  }
}
