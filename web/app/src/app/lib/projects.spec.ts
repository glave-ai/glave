import { ApiError, ErrorCode } from '@glave/client';

import {
  type ProjectDraft,
  createParams,
  crumbs,
  fallbackName,
  folderCount,
  folderGone,
  folderName,
  isProjectStart,
  joinPath,
  projectProblem,
  repositoryName,
} from './projects';
import { SOMETHING_WRONG } from './text';

const DRAFT: ProjectDraft = { name: '', start: 'folder', folder: null, url: '' };

describe('isProjectStart', () => {
  it('knows the three places a project starts from', () => {
    for (const start of ['folder', 'repository', 'empty']) {
      expect(isProjectStart(start)).toBe(true);
    }
    expect(isProjectStart('space')).toBe(false);
    expect(isProjectStart(null)).toBe(false);
  });
});

describe('folderName', () => {
  it('is the last name in the path', () => {
    expect(folderName('/Users/ana/code/app')).toBe('app');
    expect(folderName('/Users/ana/code/app/')).toBe('app');
    expect(folderName('/')).toBe('');
  });
});

describe('repositoryName', () => {
  it('is the folder git would clone the repository into', () => {
    expect(repositoryName('https://github.com/acme/app.git')).toBe('app');
    expect(repositoryName(' https://git.example.com:8443/team/web-site/ ')).toBe('web-site');
    expect(repositoryName('https://github.com/acme/app?tab=readme#top')).toBe('app');
  });

  it('reads an address still being typed, or not yet right, loosely', () => {
    expect(repositoryName('github.com/acme/notes')).toBe('notes');
    expect(repositoryName('git@github.com:acme/app.git')).toBe('app');
    expect(repositoryName('https://')).toBe('');
    expect(repositoryName('')).toBe('');
  });
});

describe('fallbackName', () => {
  it('names a project after its folder or repository, and nothing else', () => {
    expect(fallbackName({ ...DRAFT, folder: '/Users/ana/code/app' })).toBe('app');
    expect(fallbackName(DRAFT)).toBe('');
    expect(fallbackName({ ...DRAFT, start: 'repository', url: 'https://github.com/a/b' })).toBe(
      'b',
    );
    expect(fallbackName({ ...DRAFT, start: 'empty', folder: '/Users/ana/app' })).toBe('');
    expect(fallbackName({ ...DRAFT, start: null })).toBe('');
  });
});

describe('createParams', () => {
  it('asks for the folder picked, named after it unless a name is typed', () => {
    expect(createParams(DRAFT)).toBeNull();
    const folder = { ...DRAFT, folder: '/Users/ana/code/app' };
    expect(createParams(folder)).toEqual({
      name: 'app',
      folder: { kind: 'path', path: '/Users/ana/code/app' },
    });
    expect(createParams({ ...folder, name: '  Website  ' })?.name).toBe('Website');
    expect(createParams({ ...folder, name: '   ' })?.name).toBe('app');
  });

  it('asks for a repository by its address, trimmed', () => {
    const draft: ProjectDraft = { ...DRAFT, start: 'repository' };
    expect(createParams(draft)).toBeNull();
    expect(createParams({ ...draft, name: 'Notes' })).toBeNull();
    expect(createParams({ ...draft, url: ' https://github.com/acme/app ' })).toEqual({
      name: 'app',
      folder: { kind: 'repository', url: 'https://github.com/acme/app' },
    });
  });

  it('needs a name for an empty folder, or for a project with no folder', () => {
    expect(createParams({ ...DRAFT, start: 'empty' })).toBeNull();
    expect(createParams({ ...DRAFT, start: 'empty', name: 'Ideas' })).toEqual({
      name: 'Ideas',
      folder: { kind: 'empty' },
    });
    expect(createParams({ ...DRAFT, start: null })).toBeNull();
    expect(createParams({ ...DRAFT, start: null, name: ' Ideas ' })).toEqual({ name: 'Ideas' });
  });
});

describe('crumbs', () => {
  it('goes from the top of the computer down to the folder', () => {
    expect(crumbs('/Users/ana/code')).toEqual([
      { name: '', path: '/' },
      { name: 'Users', path: '/Users' },
      { name: 'ana', path: '/Users/ana' },
      { name: 'code', path: '/Users/ana/code' },
    ]);
    expect(crumbs('/')).toEqual([{ name: '', path: '/' }]);
  });

  it('keeps a path not written from the top whole', () => {
    expect(crumbs('C:\\Users\\ana')).toEqual([{ name: 'C:\\Users\\ana', path: 'C:\\Users\\ana' }]);
  });
});

describe('joinPath', () => {
  it('puts the name inside the folder', () => {
    expect(joinPath('/Users/ana', 'code')).toBe('/Users/ana/code');
    expect(joinPath('/', 'Users')).toBe('/Users');
  });
});

describe('folderCount', () => {
  it('counts in words', () => {
    expect(folderCount(0)).toBe('no folders');
    expect(folderCount(1)).toBe('1 folder');
    expect(folderCount(12)).toBe('12 folders');
  });
});

describe('folderGone', () => {
  it('knows the refusal for a project whose folder is gone, and only that', () => {
    const gone = new ApiError({
      code: ErrorCode.NO_FOLDER,
      message: "This project's folder isn't there anymore.",
    });
    expect(folderGone(gone)).toBe(true);
    expect(folderGone(new ApiError({ code: ErrorCode.WORKSPACE, message: 'no' }))).toBe(false);
    expect(folderGone(new Error('offline'))).toBe(false);
  });
});

describe('projectProblem', () => {
  it('passes on what the server wrote for people, as a sentence', () => {
    const refused = new ApiError({
      code: ErrorCode.HOST,
      message:
        'could not clone https://github.com/acme/secret: repository not found. If the repository is private, connect its git host first',
    });
    expect(projectProblem(refused)).toBe(
      'Could not clone https://github.com/acme/secret: repository not found. If the repository is private, connect its git host first.',
    );
    expect(
      projectProblem(
        new ApiError({ code: ErrorCode.INVALID_PARAMS, message: 'there is no folder at /x' }),
      ),
    ).toBe('There is no folder at /x.');
    expect(
      projectProblem(
        new ApiError({
          code: ErrorCode.NOT_ALLOWED,
          message: '/x belongs to a project you do not work in',
        }),
      ),
    ).toBe('/x belongs to a project you do not work in.');
    expect(
      projectProblem(
        new ApiError({
          code: ErrorCode.TURN_RUNNING,
          message: 'something is running in this project: stop it first, then remove the project',
        }),
      ),
    ).toBe('Something is running in this project: stop it first, then remove the project.');
  });

  it('says only that something went wrong otherwise', () => {
    expect(
      projectProblem(new ApiError({ code: ErrorCode.INTERNAL, message: 'store: disk full' })),
    ).toBe(SOMETHING_WRONG);
    expect(projectProblem(new Error('offline'))).toBe(SOMETHING_WRONG);
  });
});
