import { Service, computed, inject, linkedSignal } from '@angular/core';
import type { ThreadId } from '@glave/client';

import { type Thought, trimBlankLines } from '../lib/conversation';
import { PanelService } from './panel.service';
import { ThreadService } from './thread.service';

/** The key of the reasoning being written now, before its message is recorded. */
export const LIVE_THOUGHT = 'live';

/** The model's reasoning shown in full: its text, and what it came to as the server says. */
export interface ShownThought {
  readonly key: string;
  readonly text: string;
  readonly thought: Thought;
}

/**
 * Which of the open thread's reasoning the panel shows in full, as the person chose its line:
 * a recorded message's, by its key, or the one being written (`LIVE_THOUGHT`), which becomes
 * its message's once that is recorded. Choosing a line shows the panel (a sheet on a narrow
 * screen); choosing the one shown again shows it no longer. Each thread view provides its own.
 */
@Service({ autoProvided: false })
export class ThoughtService {
  private readonly thread = inject(ThreadService);
  private readonly panel = inject(PanelService);
  /** The line chosen; starts again from nothing in each thread. */
  private readonly chosen = linkedSignal<ThreadId | null, string | null>({
    source: this.thread.thread,
    computation: () => null,
  });

  /** Every reasoning of the thread's recorded messages, in order. */
  private readonly recorded = computed<readonly ShownThought[]>(() =>
    this.thread
      .turns()
      .flatMap((turn) => turn.items)
      .flatMap((item) =>
        item.kind === 'message' && item.reasoning !== ''
          ? [{ key: item.key, text: item.reasoning, thought: item.thought }]
          : [],
      ),
  );

  /** The reasoning shown in full, while one is chosen; `null` otherwise. */
  readonly shown = computed<ShownThought | null>(() => {
    const chosen = this.chosen();
    if (chosen !== LIVE_THOUGHT) {
      return this.recorded().find((each) => each.key === chosen) ?? null;
    }
    const draft = this.thread.reply();
    const text = trimBlankLines(draft.reasoning);
    // Once recorded, the reasoning being written is its message's: the newest.
    return text === ''
      ? (this.recorded().at(-1) ?? null)
      : { key: LIVE_THOUGHT, text, thought: draft.thought };
  });

  /** The key of the line whose reasoning shows in full now, beside the page or over it. */
  readonly selected = computed(() => (this.panel.showing() ? (this.shown()?.key ?? null) : null));

  /** Shows the reasoning of the line `key` in full, or, when it shows already, no longer. */
  choose(key: string): void {
    if (this.selected() === key) {
      this.chosen.set(null);
      return;
    }
    this.chosen.set(key);
    this.panel.setShowing(true);
  }
}
