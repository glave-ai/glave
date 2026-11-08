import { CSP_NONCE } from '@angular/core';
import { TestBed } from '@angular/core/testing';
import type { FitAddon } from '@xterm/addon-fit';
import type { ITerminalOptions, Terminal } from '@xterm/xterm';
import { expectAccessible } from '@glave/glaze/testing';

import { TerminalsService } from '../../services/terminals.service';
import { ThreadService } from '../../services/thread.service';
import { FakeSession, provideFakeSession } from '../../testing/fake-session';
import {
  type Emulator,
  TERMINAL_EMULATOR,
  TerminalScreenComponent,
} from './terminal-screen.component';

/** What the fake emulator was given and did, for the test to read. */
interface Drawn {
  options: ITerminalOptions | null;
  written: string[];
  typed: ((data: string) => void) | null;
  /** What it was told to send as bytes, as xterm.js reports a mouse a program asked for. */
  sent: ((data: string) => void) | null;
  styles: HTMLStyleElement[];
  focused: number;
  disposed: boolean;
}

/** Nothing drawn yet. */
function nothingDrawn(): Drawn {
  return {
    options: null,
    written: [],
    typed: null,
    sent: null,
    styles: [],
    focused: 0,
    disposed: false,
  };
}

/** An emulator that draws nothing: it keeps what it is given, and adds a style as it opens. */
function fakeEmulator(drawn: Drawn): Emulator {
  class FakeTerminal {
    cols = 80;
    rows = 24;
    constructor(options: ITerminalOptions) {
      drawn.options = options;
    }
    loadAddon(): void {
      // Nothing to load.
    }
    open(element: HTMLElement): void {
      const style = element.ownerDocument.createElement('style');
      element.append(style);
      drawn.styles.push(style);
    }
    write(data: Uint8Array): void {
      drawn.written.push(new TextDecoder().decode(data));
    }
    onData(typed: (data: string) => void): { dispose: () => void } {
      drawn.typed = typed;
      return { dispose: () => undefined };
    }
    onBinary(sent: (data: string) => void): { dispose: () => void } {
      drawn.sent = sent;
      return { dispose: () => undefined };
    }
    focus(): void {
      drawn.focused += 1;
    }
    dispose(): void {
      drawn.disposed = true;
    }
  }
  class FakeFit {
    fit(): void {
      // A box of 100 columns and 30 rows.
    }
  }
  return {
    Terminal: FakeTerminal as unknown as new (options: ITerminalOptions) => Terminal,
    strings: { promptLabel: '', tooMuchOutput: '' },
    FitAddon: FakeFit as unknown as new () => FitAddon,
  };
}

async function settle(): Promise<void> {
  for (let round = 0; round < 3; round += 1) {
    TestBed.tick();
    await new Promise((resolve) => setTimeout(resolve));
  }
}

describe('TerminalScreenComponent', () => {
  it('draws what its shell prints, sends what is typed, and gives the styles it adds the nonce', async () => {
    const drawn = nothingDrawn();
    let reads = 0;
    const fake = new FakeSession()
      .answer('thread/events', { events: [] })
      .answer('terminal/write', {})
      .answer('terminal/resize', {})
      .answer('terminal/read', () => {
        reads += 1;
        return reads === 1
          ? { screen: { from: 0, next: 5, output: 'aGVsbG8=' } }
          : new Promise(() => undefined);
      });
    TestBed.configureTestingModule({
      providers: [
        ...provideFakeSession(fake),
        ThreadService,
        TerminalsService,
        { provide: TERMINAL_EMULATOR, useValue: () => Promise.resolve(fakeEmulator(drawn)) },
        { provide: CSP_NONCE, useValue: 'n0nce' },
      ],
    });
    await TestBed.inject(ThreadService).open('thread-1');
    const fixture = TestBed.createComponent(TerminalScreenComponent);
    fixture.componentRef.setInput('terminal', 't1');
    await settle();
    const root = fixture.nativeElement as HTMLElement;

    expect(drawn.written).toEqual(['hello']);
    // The keyboard's place was elsewhere: it stays there.
    expect(drawn.focused).toBe(0);
    expect(drawn.styles.map((style) => style.nonce)).toEqual(['n0nce']);
    // Only while it opened: a style made after it has none.
    expect(document.createElement('style').nonce).not.toBe('n0nce');
    expect(drawn.options?.scrollback).toBe(5_000);
    expect(root.querySelector('[data-testid="terminal-screen"]')?.getAttribute('dir')).toBe('ltr');
    const resized = fake.calls.filter((call) => call.method === 'terminal/resize');
    expect(resized.map((call) => call.params)).toEqual([
      { thread: 'thread-1', terminal: 't1', size: { cols: 80, rows: 24 } },
    ]);

    drawn.typed?.('ls\r');
    // Bytes as xterm.js gives them, one to a character, none of them UTF-8's.
    drawn.sent?.('\x1b[M\xff!');
    await settle();
    const typed = fake.calls.filter((call) => call.method === 'terminal/write');
    expect(typed.map((call) => call.params)).toEqual([
      { thread: 'thread-1', terminal: 't1', input: 'bHMN' },
      { thread: 'thread-1', terminal: 't1', input: 'G1tN/yE=' },
    ]);
    await expectAccessible(root);
    fixture.destroy();
    expect(drawn.disposed).toBe(true);
  });

  it('sends what is typed on it before the emulator loads, in order, then gives it the keyboard', async () => {
    const drawn = nothingDrawn();
    let loaded: (emulator: Emulator) => void = () => undefined;
    const fake = new FakeSession()
      .answer('thread/events', { events: [] })
      .answer('terminal/write', {})
      .answer('terminal/resize', {})
      .answer('terminal/read', () => new Promise(() => undefined));
    const load = (): Promise<Emulator> =>
      new Promise((resolve) => {
        loaded = resolve;
      });
    TestBed.configureTestingModule({
      providers: [
        ...provideFakeSession(fake),
        ThreadService,
        TerminalsService,
        { provide: TERMINAL_EMULATOR, useValue: load },
      ],
    });
    await TestBed.inject(ThreadService).open('thread-1');
    const fixture = TestBed.createComponent(TerminalScreenComponent);
    fixture.componentRef.setInput('terminal', 't1');
    await settle();
    const root = fixture.nativeElement as HTMLElement;
    const screen = root.querySelector<HTMLElement>('[data-testid="terminal-screen"]');
    const press = (key: string, more: KeyboardEventInit = {}): void => {
      screen?.dispatchEvent(new KeyboardEvent('keydown', { key, bubbles: true, ...more }));
    };
    const written = (): readonly unknown[] =>
      fake.calls.filter((call) => call.method === 'terminal/write').map((call) => call.params);
    const input = (text: string): unknown => ({
      thread: 'thread-1',
      terminal: 't1',
      input: btoa(text),
    });

    screen?.focus();
    expect(document.activeElement).toBe(screen);
    press('l');
    press('s');
    press('Backspace');
    press('Enter');
    // Neither a key that types nothing nor the browser's own shortcut.
    press('Shift');
    press('r', { metaKey: true });
    await settle();
    expect(written()).toEqual([input('l'), input('s'), input('\x7f'), input('\r')]);

    loaded(fakeEmulator(drawn));
    await settle();
    expect(drawn.focused).toBe(1);
    // Drawn, what is typed comes from the terminal alone.
    press('x');
    await settle();
    expect(written()).toHaveLength(4);
    await expectAccessible(root);
  });
});
