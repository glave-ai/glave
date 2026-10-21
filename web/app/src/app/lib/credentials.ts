import { type Ability, ApiError, type CredentialInfo, ErrorCode, type Forge } from '@glave/client';

import { sentence, somethingWrong } from './text';

/** Where Settings shows the connected accounts: a link to `/settings#accounts` lands there. */
export const ACCOUNTS_SECTION = 'accounts';

/** GitHub's own address: connecting GitHub with a token sends it here. */
export const GITHUB = 'https://github.com';

/** What each forge is called, by its own name. */
const FORGES: Readonly<Record<Exclude<Forge, 'other'>, string>> = {
  github: 'GitHub',
  gitlab: 'GitLab',
  gitea: 'Gitea',
  bitbucket: 'Bitbucket',
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
  if (login !== '' && hint !== '') {
    return $localize`:@@credentials.signedInToken:Signed in as ${login}:LOGIN:, token ending ${hint}:HINT:`;
  }
  if (login !== '') {
    return $localize`:@@credentials.signedIn:Signed in as ${login}:LOGIN:`;
  }
  return hint === ''
    ? $localize`:@@credentials.connected:Connected`
    : $localize`:@@credentials.token:Token ending ${hint}:HINT:`;
}

/** A thing glave can do with an account, in plain words; GitLab calls pull requests merge requests. */
function abilityText(ability: Ability, gitlab: boolean): string {
  switch (ability) {
    case 'read':
      return $localize`:@@credentials.can.read:Read and clone your repositories`;
    case 'push':
      return $localize`:@@credentials.can.push:Push branches`;
    case 'pull_requests':
      return gitlab
        ? $localize`:@@credentials.can.mergeRequests:Open and comment on merge requests`
        : $localize`:@@credentials.can.pullRequests:Open and comment on pull requests`;
    case 'issues':
      return $localize`:@@credentials.can.issues:Open and comment on issues`;
  }
}

/** What glave can do with an account: the list, or a note when there is none to give. */
export interface AccountAbilities {
  readonly abilities: readonly string[];
  readonly note: string | null;
}

/** What glave can do with an account, only as far as its host said. */
export function accountAbilities(account: CredentialInfo): AccountAbilities {
  const can = account.can ?? null;
  if (can === null) {
    // The host did not say what its token may do: nothing is claimed.
    return {
      abilities: [],
      note: $localize`:@@credentials.can.unsaid:What glave can do with it depends on the access you gave its token.`,
    };
  }
  if (can.length === 0) {
    return {
      abilities: [],
      note: $localize`:@@credentials.can.none:Nothing with your repositories yet.`,
    };
  }
  const gitlab = account.forge === 'gitlab';
  return { abilities: can.map((ability) => abilityText(ability, gitlab)), note: null };
}

/** What to tell the person when connecting failed, in a sentence they can act on. */
export function connectProblem(error: unknown): string {
  if (!(error instanceof ApiError)) {
    return somethingWrong();
  }
  switch (error.code) {
    case ErrorCode.HOST:
    case ErrorCode.INVALID_PARAMS:
    case ErrorCode.NOT_ALLOWED:
    case ErrorCode.NO_CREDENTIAL:
      return sentence(error.message);
    case ErrorCode.NOT_AVAILABLE:
      return $localize`:@@credentials.notHere:Accounts are connected on the computer glave runs on.`;
    default:
      return somethingWrong();
  }
}
