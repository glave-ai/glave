import { DOCUMENT } from '@angular/common';
import { DestroyRef, Service, computed, inject, signal } from '@angular/core';
import type { Space, SpaceCreateParams, SpaceId, ThreadId, ThreadSummary } from '@glave/client';

import type { SpaceWithThreads } from '../lib/nav';
import { SessionService } from './session.service';

/** How often the lists are read again while the app is open, in milliseconds. */
export const REFRESH_MS = 10_000;

/**
 * The spaces the person works in and each one's latest threads, read again every few seconds
 * and after anything the app changes, so the side nav shows what is running and waiting.
 */
@Service()
export class SpacesService {
  private readonly session = inject(SessionService);
  private readonly document = inject(DOCUMENT);
  /** The read under way, which a second asks join rather than start another. */
  private pending: Promise<void> | null = null;
  private readonly lists = signal<readonly SpaceWithThreads[]>([]);
  private readonly failed = signal<string | null>(null);
  private readonly done = signal(false);

  /** Projects first, then the personal space; each with its threads, newest first. */
  readonly spaces = computed(() =>
    [...this.lists()].sort((a, b) => kindOrder(a.space) - kindOrder(b.space)),
  );
  readonly error = this.failed.asReadonly();
  readonly loaded = this.done.asReadonly();

  constructor() {
    // Polling skips a tab no one is looking at, and a read still under way.
    const timer = setInterval(() => {
      if (this.session.signedIn() && !this.document.hidden && this.pending === null) {
        void this.refresh();
      }
    }, REFRESH_MS);
    inject(DestroyRef).onDestroy(() => {
      clearInterval(timer);
    });
  }

  /** Reads the spaces and their threads again, or joins the read under way. */
  refresh(): Promise<void> {
    this.pending ??= this.readAll().finally(() => {
      this.pending = null;
    });
    return this.pending;
  }

  /**
   * Makes a project (`space/create`) and reads the lists again so it shows: after the read under
   * way, if there is one, since that one began before the project was made. Answers the project
   * as the server made it, whose name may differ from the one asked for.
   */
  async create(params: SpaceCreateParams): Promise<Space> {
    const { space } = await this.session.call('space/create', params);
    if (this.pending !== null) {
      await this.pending;
    }
    await this.refresh();
    return space;
  }

  private async readAll(): Promise<void> {
    try {
      const { spaces } = await this.session.call('space/list', {});
      const lists = await Promise.all(
        spaces.map(async (space) => {
          const { threads } = await this.session.call('thread/list', {
            space: space.id,
            limit: 30,
          });
          return { space, threads };
        }),
      );
      this.lists.set(lists);
      this.failed.set(null);
    } catch (error: unknown) {
      this.failed.set(error instanceof Error ? error.message : String(error));
    } finally {
      this.done.set(true);
    }
  }

  /** The summary of `thread`, if a listed space has it. */
  thread(id: ThreadId): ThreadSummary | undefined {
    for (const { threads } of this.lists()) {
      const found = threads.find((thread) => thread.id === id);
      if (found !== undefined) {
        return found;
      }
    }
    return undefined;
  }

  /** The space that holds `thread`, if a listed space has it. */
  spaceOf(id: ThreadId): Space | undefined {
    return this.lists().find(({ threads }) => threads.some((thread) => thread.id === id))?.space;
  }

  /** The space new chats start in: the person's own (Chats), never a project. */
  readonly home = computed<SpaceId | undefined>(
    () => this.spaces().find(({ space }) => space.kind !== 'project')?.space.id,
  );
}

function kindOrder(space: Space): number {
  return space.kind === 'project' ? 0 : 1;
}
