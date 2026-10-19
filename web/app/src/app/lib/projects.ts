import { ApiError, ErrorCode, type ProjectCreateParams } from '@glave/client';

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

/** The longest name `project/create` takes. */
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

/**
 * A folder on the way down to another, to go up to: its name (empty for the top of the
 * computer, the drive's for a Windows drive's root) and path.
 */
export interface Crumb {
  readonly name: string;
  readonly path: string;
}

/** A path written from the top, taken apart: its top, the folders under it, what joins them. */
interface Parts {
  /** The top: `/`, named `''`, or a Windows drive's root such as `C:\`, named `C:`. */
  readonly top: Crumb;
  readonly names: readonly string[];
  readonly separator: '/' | '\\';
}

/** A Windows drive's root, `C:\` (or `C:/`), capturing the drive's name. */
const DRIVE = /^([a-z]:)[\\/]/iu;

/** `path` taken apart, or `null` for a path not written from the top. */
function parts(path: string): Parts | null {
  const named = (name: string): boolean => name !== '';
  if (path.startsWith('/')) {
    return { top: { name: '', path: '/' }, names: path.split('/').filter(named), separator: '/' };
  }
  const drive = DRIVE.exec(path)?.[1];
  if (drive === undefined) {
    return null;
  }
  const top = { name: drive, path: `${drive}\\` };
  const names = path.slice(top.path.length).split(/[\\/]/u).filter(named);
  return { top, names, separator: '\\' };
}

/**
 * The last name in the path `path`: `app` for `/Users/ana/code/app` or `C:\code\app`; the
 * drive's name for a drive's root (`C:`), and empty for the top of the computer.
 */
export function folderName(path: string): string {
  const parted = parts(path);
  return parted === null ? path : (parted.names.at(-1) ?? parted.top.name);
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
export function fallbackName(draft: Omit<ProjectDraft, 'name'>): string {
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

/** What `project/create` is asked for the draft, or `null` while it lacks something it needs. */
export function createParams(draft: ProjectDraft): ProjectCreateParams | null {
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
 * gives it, each with its own path; on Windows, a drive's root is the top. A path not written
 * from the top is one crumb.
 */
export function crumbs(path: string): readonly Crumb[] {
  const parted = parts(path);
  if (parted === null) {
    return [{ name: path, path }];
  }
  const { top, names, separator } = parted;
  return [
    top,
    ...names.map((name, index) => ({
      name,
      path: `${top.path}${names.slice(0, index + 1).join(separator)}`,
    })),
  ];
}

/** The path of the folder `name` inside the folder at `path`, joined as `path` is written. */
export function joinPath(path: string, name: string): string {
  const separator = parts(path)?.separator ?? '/';
  return path.endsWith(separator) ? `${path}${name}` : `${path}${separator}${name}`;
}

/** How many folders a folder holds, in words. */
export function folderCount(count: number): string {
  if (count === 0) {
    return 'no folders';
  }
  return count === 1 ? '1 folder' : `${String(count)} folders`;
}

/** Whether the server refused because the project's folder isn't there anymore. */
export function folderGone(error: unknown): boolean {
  return error instanceof ApiError && error.code === ErrorCode.NO_FOLDER;
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
    case ErrorCode.NO_FOLDER:
    case ErrorCode.TURN_RUNNING:
      return sentence(error.message);
    default:
      return SOMETHING_WRONG;
  }
}
