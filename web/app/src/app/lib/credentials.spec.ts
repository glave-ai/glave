import { ApiError, type CredentialInfo, ErrorCode } from '@glave/client';

import { accountAbilities, accountDetail, accountName, connectProblem } from './credentials';
import { somethingWrong } from './text';

const GITHUB: CredentialInfo = {
  id: '01900000-0000-7000-8000-000000000401',
  forge: 'github',
  url: 'https://github.com',
  login: 'octocat',
  hint: 'c3d4',
};

/** What the list says when the host did not say what its token may do, or said nothing. */
const UNSAID = 'What glave can do with it depends on the access you gave its token.';
const NONE = 'Nothing with your repositories yet.';

describe('accountName', () => {
  it('names GitHub plainly, and other hosts by their name and forge', () => {
    expect(accountName(GITHUB)).toBe('GitHub');
    expect(accountName({ ...GITHUB, forge: 'gitlab', url: 'https://gitlab.example.com' })).toBe(
      'gitlab.example.com (GitLab)',
    );
    expect(accountName({ ...GITHUB, forge: 'other', url: 'https://git.example.com' })).toBe(
      'git.example.com',
    );
  });
});

describe('accountDetail', () => {
  it('says whom the account is for and how its token ends, when known', () => {
    expect(accountDetail(GITHUB)).toBe('Signed in as octocat, token ending c3d4');
    expect(accountDetail({ ...GITHUB, login: null })).toBe('Token ending c3d4');
    expect(accountDetail({ ...GITHUB, login: null, hint: null })).toBe('Connected');
  });
});

describe('accountAbilities', () => {
  it('lists what the host said glave can do, and claims nothing it did not say', () => {
    expect(accountAbilities({ ...GITHUB, can: ['read', 'push', 'pull_requests'] })).toEqual({
      abilities: [
        'Read and clone your repositories',
        'Push branches',
        'Open and comment on pull requests',
      ],
      note: null,
    });
    expect(
      accountAbilities({ ...GITHUB, forge: 'gitlab', can: ['pull_requests', 'issues'] }),
    ).toEqual({
      abilities: ['Open and comment on merge requests', 'Open and comment on issues'],
      note: null,
    });
    expect(accountAbilities(GITHUB)).toEqual({ abilities: [], note: UNSAID });
    expect(accountAbilities({ ...GITHUB, can: null })).toEqual({
      abilities: [],
      note: UNSAID,
    });
    expect(accountAbilities({ ...GITHUB, can: [] })).toEqual({
      abilities: [],
      note: NONE,
    });
  });
});

describe('connectProblem', () => {
  it('passes the server’s own sentence on, and a plain one otherwise', () => {
    const refused = new ApiError({
      code: ErrorCode.HOST,
      message:
        'the git host at git.example.com did not accept that token: check that you copied all of it',
    });
    expect(connectProblem(refused)).toBe(
      'The git host at git.example.com did not accept that token: check that you copied all of it.',
    );
    const elsewhere = new ApiError({ code: ErrorCode.NOT_AVAILABLE, message: 'x' });
    expect(connectProblem(elsewhere)).toBe('Accounts are connected on the computer glave runs on.');
    expect(connectProblem(new Error('boom'))).toBe(somethingWrong());
    expect(connectProblem(new ApiError({ code: ErrorCode.INTERNAL, message: 'x' }))).toBe(
      somethingWrong(),
    );
  });
});
