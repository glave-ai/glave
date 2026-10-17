import { ApiError, ErrorCode, type SpaceCreateParams } from '@glave/client';

import { SOMETHING_WRONG, sentence } from './text';

/**
 * Where a new project's files come from: a folder already on this computer, a repository
 * cloned onto it, or a new, empty folder glave makes.
 */
export type ProjectStart = 'folder' | 'repository' | 'empty';

/** Whether `value` is one of the places a project can start from. */
export function isProjectStart(value: unknown): value is ProjectStart {
  return value === 'folder' || value === 'repository' || value === 'empty';
}

/** The longest name `space/create` takes. */
export const NAME_LIMIT = 200;

/** What the person has asked for so far in New project. */
export interface ProjectDraft {
  /** The name, as typed. */
  readonly name: string;
  /** Where its files come from; `null` for a project with no folder. */
  readonly start: ProjectStart | null;
  /** The folder picked, for `folder`. */
  readonly folder: string | null;
  /** The repository's address, as typed, for `repository`. */
  readonly url: string;
}

/** A folder on the way down to another, to go up to: its name (empty for the top) and path. */
export interface Crumb {
  readonly name: string;
  readonly path: string;
}

/** The last name in the path `path`: `app` for `/Users/ana/code/app`; empty for the top. */
export function folderName(path: string): string {
  return (
    path
      .split('/')
      .filter((name) => name !== '')
      .at(-1) ?? ''
  );
}

/**
 * The repository's name in the address `url`, as git names the folder it clones into: `app`
 * for `https://github.com/acme/app.git`. Read loosely, since the person may still be typing;
 * the server says whether the address will do.
 */
export function repositoryName(url: string): string {
  const path = url
    .trim()
    .replace(/^[a-z][a-z0-9+.-]*:\/\//iu, '')
    .replace(/[?#].*$/u, '');
  const last =
    path
      .split(/[/:]/u)
      .filter((name) => name !== '')
      .at(-1) ?? '';
  return last.replace(/\.git$/iu, '');
}

/** The name a project takes when none is typed: its folder's or its repository's, if it has one. */
export function fallbackName(draft: ProjectDraft): string {
  switch (draft.start) {
    case 'folder':
      return draft.folder === null ? '' : folderName(draft.folder);
    case 'repository':
      return repositoryName(draft.url);
    case 'empty':
    case null:
      return '';
  }
}

/** What `space/create` is asked for the draft, or `null` while it lacks something it needs. */
export function createParams(draft: ProjectDraft): SpaceCreateParams | null {
  const typed = draft.name.trim();
  const name = typed === '' ? fallbackName(draft) : typed;
  if (name === '') {
    return null;
  }
  switch (draft.start) {
    case null:
      return { name };
    case 'folder':
      return draft.folder === null ? null : { name, folder: { kind: 'path', path: draft.folder } };
    case 'repository': {
      const url = draft.url.trim();
      return url === '' ? null : { name, folder: { kind: 'repository', url } };
    }
    case 'empty':
      return { name, folder: { kind: 'empty' } };
  }
}

/**
 * The folders from the top of this computer down to `path`, an absolute path as `folder/list`
 * gives it, each with its own path. A path not written from the top is one crumb.
 */
export function crumbs(path: string): readonly Crumb[] {
  if (!path.startsWith('/')) {
    return [{ name: path, path }];
  }
  const names = path.split('/').filter((name) => name !== '');
  return [
    { name: '', path: '/' },
    ...names.map((name, index) => ({ name, path: `/${names.slice(0, index + 1).join('/')}` })),
  ];
}

/** The path of the folder `name` inside the folder at `path`. */
export function joinPath(path: string, name: string): string {
  return path.endsWith('/') ? `${path}${name}` : `${path}/${name}`;
}

/** How many folders a folder holds, in words. */
export function folderCount(count: number): string {
  if (count === 0) {
    return 'no folders';
  }
  return count === 1 ? '1 folder' : `${String(count)} folders`;
}

/**
 * What to tell the person when a project could not be made or a folder listed: the server's
 * own words where it wrote them for people (a refused clone says what git said), else the
 * plain failure.
 */
export function projectProblem(error: unknown): string {
  if (!(error instanceof ApiError)) {
    return SOMETHING_WRONG;
  }
  switch (error.code) {
    case ErrorCode.HOST:
    case ErrorCode.INVALID_PARAMS:
    case ErrorCode.NOT_ALLOWED:
      return sentence(error.message);
    default:
      return SOMETHING_WRONG;
  }
}
