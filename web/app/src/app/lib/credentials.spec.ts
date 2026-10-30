import { ApiError, type CredentialInfo, ErrorCode } from '@glave/client';

import { accountDetail, accountName, connectProblem, hostOf } from './credentials';
import { somethingWrong } from './text';

const GITHUB: CredentialInfo = {
  id: '01900000-0000-7000-8000-000000000401',
  forge: 'github',
  url: 'https://github.com',
  login: 'octocat',
};

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
  it('says whom the account is for when the host said, and never how its token ends', () => {
    expect(accountDetail(GITHUB)).toBe('Signed in as octocat');
    const { id, forge, url } = GITHUB;
    expect(accountDetail({ id, forge, url })).toBe('Connected');
  });
});

describe('hostOf', () => {
  it('reads the host an address names', () => {
    expect(hostOf(' https://gitlab.com/ ')).toBe('gitlab.com');
    expect(hostOf('git.example.com:8443/path')).toBe('git.example.com:8443');
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
