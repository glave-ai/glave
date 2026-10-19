import { DOCUMENT } from '@angular/common';
import { DestroyRef, Service, computed, inject, signal } from '@angular/core';
import type {
  Project,
  ProjectCreateParams,
  ProjectId,
  ThreadId,
  ThreadSummary,
} from '@glave/client';

import type { ProjectWithThreads } from '../lib/nav';
import { SessionService } from './session.service';

/** How often the lists are read again while the app is open, in milliseconds. */
export const REFRESH_MS = 10_000;

/**
 * The projects the person works in and each one's latest threads, read again every few seconds
 * and after anything the app changes, so the side nav shows what is running and waiting.
 */
@Service()
export class ProjectsService {
  private readonly session = inject(SessionService);
  private readonly document = inject(DOCUMENT);
  /** The read under way, which a second asks join rather than start another. */
  private pending: Promise<void> | null = null;
  private readonly lists = signal<readonly ProjectWithThreads[]>([]);
  private readonly failed = signal<string | null>(null);
  private readonly done = signal(false);

  /** Projects first, then the personal project; each with its threads, newest first. */
  readonly projects = computed(() =>
    [...this.lists()].sort((a, b) => kindOrder(a.project) - kindOrder(b.project)),
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

  /** Reads the projects and their threads again, or joins the read under way. */
  refresh(): Promise<void> {
    this.pending ??= this.readAll().finally(() => {
      this.pending = null;
    });
    return this.pending;
  }

  /**
   * Makes a project (`project/create`) and reads the lists again so it shows: after the read under
   * way, if there is one, since that one began before the project was made. Answers the project
   * as the server made it, whose name may differ from the one asked for.
   */
  async create(params: ProjectCreateParams): Promise<Project> {
    const { project } = await this.session.call('project/create', params);
    await this.reread();
    return project;
  }

  /**
   * Gives a project a new name (`project/rename`) and reads the lists again so it shows, as
   * `create` does. Throws what the server refused.
   */
  async rename(project: ProjectId, name: string): Promise<void> {
    await this.session.call('project/rename', { project, name });
    await this.reread();
  }

  /**
   * Gives a project whose folder isn't there anymore its folder's new place, the folder at
   * `path` on this computer (`project/folder/set`), and reads the lists again. Throws what the
   * server refused.
   */
  async findFolder(project: ProjectId, path: string): Promise<void> {
    await this.session.call('project/folder/set', { project, path });
    await this.reread();
  }

  /**
   * Removes a project (`project/remove`): its threads go to the archive. Reads the lists again,
   * which no longer have it. Throws what the server refused.
   */
  async remove(project: ProjectId): Promise<void> {
    await this.session.call('project/remove', { project });
    await this.reread();
  }

  /** What the server says of the project `project` when its folder isn't there anymore. */
  gone(project: ProjectId): string | null {
    return this.lists().find((entry) => entry.project.id === project)?.gone ?? null;
  }

  /** Reads the lists again after a change: after the read under way, which began before it. */
  private async reread(): Promise<void> {
    if (this.pending !== null) {
      await this.pending;
    }
    await this.refresh();
  }

  private async readAll(): Promise<void> {
    try {
      const { projects, gone = [] } = await this.session.call('project/list', {});
      const said = new Map(gone.map((each) => [each.project, each.message]));
      const lists = await Promise.all(
        projects.map(async (project): Promise<ProjectWithThreads> => {
          const { threads } = await this.session.call('thread/list', {
            project: project.id,
            limit: 30,
          });
          const message = said.get(project.id);
          return message === undefined ? { project, threads } : { project, threads, gone: message };
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

  /** The summary of `thread`, if a listed project has it. */
  thread(id: ThreadId): ThreadSummary | undefined {
    for (const { threads } of this.lists()) {
      const found = threads.find((thread) => thread.id === id);
      if (found !== undefined) {
        return found;
      }
    }
    return undefined;
  }

  /** The project that holds `thread`, if a listed project has it. */
  projectOf(id: ThreadId): Project | undefined {
    return this.lists().find(({ threads }) => threads.some((thread) => thread.id === id))?.project;
  }

  /** The project new chats start in: the person's own (Chats), never a project. */
  readonly home = computed<ProjectId | undefined>(
    () => this.projects().find(({ project }) => project.kind !== 'project')?.project.id,
  );
}

function kindOrder(project: Project): number {
  return project.kind === 'project' ? 0 : 1;
}
