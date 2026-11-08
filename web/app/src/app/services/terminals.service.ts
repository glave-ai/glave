import { DOCUMENT, Service, computed, inject, linkedSignal } from '@angular/core';
import {
  ApiError,
  ErrorCode,
  type TerminalId,
  type TerminalInfo,
  type ThreadId,
} from '@glave/client';

import { base64Of } from '../lib/attachments';
import { fromBase64url } from '../lib/passkeys';
import { sentence, somethingWrong } from '../lib/text';
import { SessionService } from './session.service';
import { ThreadService } from './thread.service';

/** How long one read waits for the shell to print: the server waits at most this long. */
const READ_WAIT_MS = 20_000;

/** How long a read that failed waits before the next, doubling to at most the longest. */
const RETRY_FIRST_MS = 1_000;
const RETRY_LONGEST_MS = 30_000;

/** How often the thread's terminals are listed again while the tab shows. */
const RELIST_MS = 15_000;

/** The most bytes one write takes, as the server says (`TERMINAL_CHUNK`): a paste goes in pieces. */
const WRITE_MOST = 64 * 1024;

/** `bytes` in pieces of at most `most`, in order. */
function piecesOf(bytes: Uint8Array, most: number): readonly Uint8Array[] {
  return Array.from({ length: Math.ceil(bytes.length / most) }, (_, index) =>
    bytes.subarray(index * most, (index + 1) * most),
  );
}

/** Resolves after `ms`, or as `stop` aborts. */
function pause(ms: number, stop: AbortSignal): Promise<void> {
  return new Promise((resolve) => {
    const timer = setTimeout(resolve, ms);
    stop.addEventListener(
      'abort',
      () => {
        clearTimeout(timer);
        resolve();
      },
      { once: true },
    );
  });
}

/** Resolves once `document` is shown again, or as `stop` aborts. */
function shownAgain(document: Document, stop: AbortSignal): Promise<void> {
  return new Promise((resolve) => {
    document.addEventListener('visibilitychange', () => resolve(), { once: true, signal: stop });
    stop.addEventListener('abort', () => resolve(), { once: true });
  });
}

/** How many columns and rows a terminal has. */
export interface Size {
  readonly cols: number;
  readonly rows: number;
}

/** What is known of the open thread's terminals. */
interface State {
  readonly terminals: readonly TerminalInfo[];
  /** The one shown. */
  readonly shown: TerminalId | null;
  /** Those whose shell ended. */
  readonly ended: ReadonlySet<TerminalId>;
  /** Why a terminal could not be opened or reached, in words for the person. */
  readonly problem: string | null;
  readonly opening: boolean;
}

const NOTHING_YET: State = {
  terminals: [],
  shown: null,
  ended: new Set(),
  problem: null,
  opening: false,
};

/** What to tell the person when a terminal call failed: the server's words, else the plain failure. */
function problemOf(error: unknown): string {
  return error instanceof ApiError ? sentence(error.message) : somethingWrong();
}

/**
 * The open thread's terminals (`terminal/*`): the person's own shell in its folder, on their own
 * computer. Listed when the Terminal tab first shows and again every so often while it does, one
 * opened when there are none, another on asking; a screen reads what its terminal printed from
 * its start, as much as its computer keeps, so a terminal shown again shows what it printed while
 * hidden, and the keys typed go to it in order. Nothing of it is kept in the page but what its
 * screen shows. Each thread view provides its own.
 */
@Service({ autoProvided: false })
export class TerminalsService {
  private readonly session = inject(SessionService);
  private readonly thread = inject(ThreadService);
  private readonly document = inject(DOCUMENT);
  /** Starts again from nothing in each thread. */
  private readonly state = linkedSignal<ThreadId | null, State>({
    source: this.thread.thread,
    computation: () => NOTHING_YET,
  });
  /** The keys typed into each terminal, sent one call after another. */
  private readonly sending = new Map<TerminalId, Promise<void>>();
  /**
   * Each terminal whose size is being sent, and the size it took since, if any, sent once that
   * call is done: only the latest of a drag's many sizes goes.
   */
  private readonly resizing = new Map<TerminalId, Size | null>();

  readonly terminals = computed(() => this.state().terminals);
  readonly shown = computed(() => {
    const { terminals, shown } = this.state();
    return terminals.find((terminal) => terminal.id === shown) ?? null;
  });
  readonly ended = computed(() => {
    const shown = this.state().shown;
    return shown !== null && this.state().ended.has(shown);
  });
  readonly problem = computed(() => this.state().problem);
  readonly opening = computed(() => this.state().opening);

  /** Reads the thread's terminals, and opens one of `size` when it has none. */
  async start(size: Size): Promise<void> {
    const thread = this.thread.thread();
    if (thread === null || this.state().terminals.length > 0) {
      return;
    }
    try {
      const { terminals } = await this.session.call('terminal/list', { thread });
      if (this.thread.thread() !== thread) {
        return;
      }
      if (terminals.length === 0) {
        await this.open(size);
        return;
      }
      this.state.update((state) => ({
        ...state,
        terminals,
        shown: terminals.at(-1)?.id ?? null,
        problem: null,
      }));
    } catch (error: unknown) {
      this.failed(thread, error);
    }
  }

  /** Opens another terminal of `size`, and shows it. */
  async open(size: Size): Promise<void> {
    const thread = this.thread.thread();
    if (thread === null || this.opening()) {
      return;
    }
    this.state.update((state) => ({ ...state, opening: true, problem: null }));
    try {
      const { terminal } = await this.session.call('terminal/open', { thread, size });
      if (this.thread.thread() === thread) {
        this.state.update((state) => ({
          ...state,
          terminals: [...state.terminals, terminal],
          shown: terminal.id,
          opening: false,
        }));
      }
    } catch (error: unknown) {
      this.failed(thread, error);
      // Refused, as past the most a thread keeps: show those it has.
      await this.relist(thread);
    }
  }

  /**
   * Lists the open thread's terminals again every so often until `stop` aborts, while the page
   * is shown, so one that ended or went with its computer shows so here.
   */
  async watch(stop: AbortSignal): Promise<void> {
    // Read as it is now each time: it can be stopped while it waits.
    const stopped = (): boolean => stop.aborted;
    while (!stopped()) {
      await pause(RELIST_MS, stop);
      if (this.document.hidden) {
        await shownAgain(this.document, stop);
        continue;
      }
      const thread = this.thread.thread();
      if (thread !== null && !stopped()) {
        await this.relist(thread);
      }
    }
  }

  /** Lists `thread`'s terminals again, keeping the one shown where it is still there. */
  private async relist(thread: ThreadId): Promise<void> {
    try {
      const { terminals } = await this.session.call('terminal/list', { thread });
      if (this.thread.thread() !== thread) {
        return;
      }
      const ended = terminals.filter((each) => each.ended === true).map((each) => each.id);
      this.state.update((state) => ({
        ...state,
        terminals,
        shown: terminals.some((each) => each.id === state.shown)
          ? state.shown
          : (terminals.at(-1)?.id ?? null),
        ended: new Set([...state.ended, ...ended]),
      }));
    } catch (error: unknown) {
      this.failed(thread, error);
    }
  }

  /** Shows `terminal`. */
  show(terminal: TerminalId): void {
    this.state.update((state) => ({ ...state, shown: terminal }));
  }

  /** Closes `terminal`, which ends its shell, and shows the one before it. */
  async close(terminal: TerminalId): Promise<void> {
    const thread = this.thread.thread();
    if (thread === null) {
      return;
    }
    const left = this.state().terminals.filter((each) => each.id !== terminal);
    this.state.update((state) => ({
      ...state,
      terminals: left,
      shown: state.shown === terminal ? (left.at(-1)?.id ?? null) : state.shown,
    }));
    try {
      await this.session.call('terminal/close', { thread, terminal });
    } catch (error: unknown) {
      // One already gone is closed, as asked.
      if (!(error instanceof ApiError && error.code === ErrorCode.NO_TERMINAL)) {
        this.failed(thread, error);
      }
    }
  }

  /**
   * Hands `print` what `terminal` prints, from its start, until `stop` aborts or its shell ends,
   * which it then says. It reads only while the page is shown, so a hidden tab never keeps the
   * terminal or its computer in use; a read that fails is tried again, a second later, then
   * longer, to half a minute, saying why meanwhile; one that finds the terminal gone stops.
   */
  async follow(
    terminal: TerminalId,
    print: (bytes: Uint8Array) => void,
    stop: AbortSignal,
  ): Promise<void> {
    const thread = this.thread.thread();
    // Read as it is now each time: a read can be stopped while it waits.
    const stopped = (): boolean => stop.aborted;
    let from = 0;
    let retry = RETRY_FIRST_MS;
    while (thread !== null && !stopped()) {
      if (this.document.hidden) {
        await shownAgain(this.document, stop);
        continue;
      }
      try {
        const params = { thread, terminal, from, wait_ms: READ_WAIT_MS };
        const { screen } = await this.session.call('terminal/read', params, stop);
        if (stopped()) {
          return;
        }
        const bytes = fromBase64url(screen.output);
        if (bytes !== null && bytes.length > 0) {
          print(bytes);
        }
        from = screen.next;
        retry = RETRY_FIRST_MS;
        this.state.update((state) => ({ ...state, problem: null }));
        if (screen.ended === true) {
          this.state.update((state) => ({ ...state, ended: new Set([...state.ended, terminal]) }));
          return;
        }
      } catch (error: unknown) {
        if (stopped()) {
          return;
        }
        this.failed(thread, error);
        if (error instanceof ApiError && error.code === ErrorCode.NO_TERMINAL) {
          return;
        }
        await pause(retry, stop);
        retry = Math.min(retry * 2, RETRY_LONGEST_MS);
      }
    }
  }

  /**
   * Types `bytes` into `terminal`, after what was typed before: a paste larger than one write
   * takes goes in pieces, in order, and stops at the first that fails.
   */
  type(terminal: TerminalId, bytes: Uint8Array): void {
    const thread = this.thread.thread();
    if (thread === null) {
      return;
    }
    const inputs = piecesOf(bytes, WRITE_MOST).map(base64Of);
    const before = this.sending.get(terminal) ?? Promise.resolve();
    const sent = before.then(async () => {
      for (const input of inputs) {
        try {
          await this.session.call('terminal/write', { thread, terminal, input });
        } catch (error: unknown) {
          this.failed(thread, error);
          return;
        }
      }
    });
    this.sending.set(terminal, sent);
  }

  /**
   * Gives `terminal` another size: at once when none is being sent, else once that is done, and
   * then only the latest it took meanwhile.
   */
  async resize(terminal: TerminalId, size: Size): Promise<void> {
    const thread = this.thread.thread();
    if (thread === null) {
      return;
    }
    if (this.resizing.has(terminal)) {
      this.resizing.set(terminal, size);
      return;
    }
    let next: Size | null = size;
    while (next !== null) {
      this.resizing.set(terminal, null);
      try {
        await this.session.call('terminal/resize', { thread, terminal, size: next });
      } catch (error: unknown) {
        this.failed(thread, error);
      }
      next = this.resizing.get(terminal) ?? null;
    }
    this.resizing.delete(terminal);
  }

  /** Says why a call in `thread` failed, while it is the thread open. */
  private failed(thread: ThreadId, error: unknown): void {
    if (this.thread.thread() === thread) {
      this.state.update((state) => ({ ...state, opening: false, problem: problemOf(error) }));
    }
  }
}
