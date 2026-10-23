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

/** What the server says of the home folder, which is never a project's. */
export const HOME_REFUSED =
  'Your home folder holds your Chats, so it is no project’s folder: pick a folder inside it.';

/**
 * The folders inside `path`, as `folder/list` answers: the home folder and those above it say
 * they are no project's.
 */
export function listFolders(path: string): unknown {
  const folders = FOLDERS[path];
  if (folders === undefined) {
    return new Refusal(-32602, `there is no folder at ${path}`);
  }
  const parent = path === '/' ? null : path.slice(0, path.lastIndexOf('/')) || '/';
  const refused =
    path === HOME
      ? HOME_REFUSED
      : HOME.startsWith(path)
        ? 'This folder holds your home folder, so it is no project’s folder.'
        : null;
  return { path, parent, folders, ...(refused === null ? {} : { refused }) };
}
