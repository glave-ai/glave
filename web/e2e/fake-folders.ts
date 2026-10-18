import { Refusal } from './fake-records';

/** The folders on the person's computer, by path; the home folder is /Users/ana. */
export const HOME = '/Users/ana';
const FOLDERS: Readonly<Record<string, readonly string[]>> = {
  '/': ['Users'],
  '/Users': ['ana'],
  [HOME]: ['code', 'Documents'],
  [`${HOME}/code`]: ['glave', 'notes'],
  [`${HOME}/code/glave`]: [],
  [`${HOME}/code/notes`]: [],
  [`${HOME}/Documents`]: [],
};

/** The folders inside `path`, as `folder/list` answers. */
export function listFolders(path: string): unknown {
  const folders = FOLDERS[path];
  if (folders === undefined) {
    return new Refusal(-32602, `there is no folder at ${path}`);
  }
  const parent = path === '/' ? null : path.slice(0, path.lastIndexOf('/')) || '/';
  return { path, parent, folders };
}
