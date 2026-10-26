import type { AttachedFile, PlanBuildParams, ThreadId, TurnStartParams } from '@glave/client';

/** What a turn the thread view follows starts with: the person's message, or a plan to build. */
export type Start =
  | { readonly method: 'turn/start'; readonly params: TurnStartParams }
  | { readonly method: 'plan/build'; readonly params: PlanBuildParams };

/** A turn started by the person's message `params`. */
export function sent(params: TurnStartParams): Start {
  return { method: 'turn/start', params };
}

/** A turn that builds `thread`'s proposed plan. */
export function built(thread: ThreadId): Start {
  return { method: 'plan/build', params: { thread } };
}

/** The person's words a turn starts with, if a message starts it. */
export function promptOf(start: Start): string | null {
  return start.method === 'turn/start' ? start.params.prompt : null;
}

/** `files` as `turn/start` takes them: left out when there are none. */
export function withFiles(files: readonly AttachedFile[]): Pick<TurnStartParams, 'files'> {
  return files.length === 0 ? {} : { files };
}
