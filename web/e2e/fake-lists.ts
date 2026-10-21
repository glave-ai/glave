import type { Route } from '@playwright/test';

/** Answers a call of the session API the way the fake does. */
type Read = (method: string, params: Record<string, unknown>) => unknown;

/** A project, or a thread's summary, as the fake lists them. */
interface Listed {
  readonly id: string;
}

/**
 * The watch of the lists the app holds open (`thread/list/watch`), which the fake answers only
 * once it has something to say, as the server tells a thread's change the moment it happens:
 * then it sends the thread's summary as the lists now show it, and ends the watch, which the
 * app comes back to after reading the lists again, as it does after any watch is cut off.
 */
export class FakeLists {
  private waiting: { readonly route: Route; readonly id: number } | null = null;

  /** Lists as `read` answers the fake's calls. */
  constructor(private readonly read: Read) {}

  /** Keeps the watch the app asked for with request `id` open. */
  hold(route: Route, id: number): void {
    this.waiting = { route, id };
  }

  /** Tells the watch, if one is open, what the lists show now of `thread`. */
  async tell(thread: string): Promise<void> {
    const waiting = this.waiting;
    const summary = this.listed(thread);
    if (waiting === null || summary === undefined) {
      return;
    }
    this.waiting = null;
    const messages = [
      { jsonrpc: '2.0', method: 'thread/changed', params: { thread, summary } },
      { jsonrpc: '2.0', id: waiting.id, result: { ended: 'unwatched' } },
    ];
    await waiting.route.fulfill({
      contentType: 'text/event-stream',
      body: messages.map((message) => `data: ${JSON.stringify(message)}\n\n`).join(''),
    });
  }

  /** What the lists show of `thread`, across every project's. */
  private listed(thread: string): Listed | undefined {
    const { projects } = this.read('project/list', {}) as { readonly projects: readonly Listed[] };
    for (const project of projects) {
      const { threads } = this.read('thread/list', { project: project.id }) as {
        readonly threads: readonly Listed[];
      };
      const found = threads.find((each) => each.id === thread);
      if (found !== undefined) {
        return found;
      }
    }
    return undefined;
  }
}
