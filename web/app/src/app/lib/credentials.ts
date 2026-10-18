import { type Ability, ApiError, type CredentialInfo, ErrorCode, type Forge } from '@glave/client';

import { SOMETHING_WRONG, sentence } from './text';

/** Where Settings shows the connected accounts: a link to `/settings#accounts` lands there. */
export const ACCOUNTS_SECTION = 'accounts';

/** GitHub's own address: connecting GitHub with a token sends it here. */
export const GITHUB = 'https://github.com';

/** What each forge is called. */
const FORGES: Readonly<Record<Forge, string>> = {
  github: 'GitHub',
  gitlab: 'GitLab',
  gitea: 'Gitea',
  bitbucket: 'Bitbucket',
  other: 'Git host',
};

/** How a connected account shows in a list: GitHub, or the host's name with its forge. */
export function accountName(account: CredentialInfo): string {
  if (account.url === GITHUB) {
    return FORGES.github;
  }
  const host = account.url.replace(/^https?:\/\//u, '');
  return account.forge === 'other' ? host : `${host} (${FORGES[account.forge]})`;
}

/** Whom an account is signed in as, and the end of its token, as far as they are known. */
export function accountDetail(account: CredentialInfo): string {
  const login = account.login ?? '';
  const hint = account.hint ?? '';
  const parts = [
    login === '' ? '' : `Signed in as ${login}`,
    hint === '' ? '' : `token ending ${hint}`,
  ].filter((part) => part !== '');
  const detail = parts.join(', ');
  return detail === '' ? 'Connected' : `${detail.charAt(0).toUpperCase()}${detail.slice(1)}`;
}

/** Each thing glave can do with an account, in plain words. */
const ABILITIES: Readonly<Record<Ability, string>> = {
  read: 'Read and clone your repositories',
  push: 'Push branches',
  pull_requests: 'Open and comment on pull requests',
  issues: 'Open and comment on issues',
};

/** GitLab's name for a pull request. */
const MERGE_REQUESTS = 'Open and comment on merge requests';

/** What to say when the host did not say what its token may do: nothing is claimed. */
export const ABILITIES_UNSAID =
  'What glave can do with it depends on the access you gave its token.';

/** What to say when the host said its token may do nothing with repositories yet. */
export const ABILITIES_NONE = 'Nothing with your repositories yet.';

/** What glave can do with an account: the list, or a note when there is none to give. */
export interface AccountAbilities {
  readonly abilities: readonly string[];
  readonly note: string | null;
}

/** What glave can do with an account, only as far as its host said. */
export function accountAbilities(account: CredentialInfo): AccountAbilities {
  const can = account.can ?? null;
  if (can === null) {
    return { abilities: [], note: ABILITIES_UNSAID };
  }
  if (can.length === 0) {
    return { abilities: [], note: ABILITIES_NONE };
  }
  const gitlab = account.forge === 'gitlab';
  const abilities = can.map((ability) =>
    gitlab && ability === 'pull_requests' ? MERGE_REQUESTS : ABILITIES[ability],
  );
  return { abilities, note: null };
}

/** What to tell the person when removing an account failed. */
export const REMOVE_FAILED = 'glave could not remove that account. Try again in a moment.';

/** What to tell the person when connecting failed, in a sentence they can act on. */
export function connectProblem(error: unknown): string {
  if (!(error instanceof ApiError)) {
    return SOMETHING_WRONG;
  }
  switch (error.code) {
    case ErrorCode.HOST:
    case ErrorCode.INVALID_PARAMS:
    case ErrorCode.NOT_ALLOWED:
    case ErrorCode.NO_CREDENTIAL:
      return sentence(error.message);
    case ErrorCode.NOT_AVAILABLE:
      return 'Accounts are connected on the computer glave runs on.';
    default:
      return SOMETHING_WRONG;
  }
}
