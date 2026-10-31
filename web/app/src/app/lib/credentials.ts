import { ApiError, type CredentialInfo, ErrorCode, type Forge } from '@glave/client';

import { sentence, somethingWrong } from './text';

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

/** Whom an account is signed in as, as far as the host said. */
export function accountDetail(account: CredentialInfo): string {
  if (account.expired === true) {
    return $localize`:@@credentials.expired:Your sign-in expired`;
  }
  const login = account.login ?? '';
  return login === ''
    ? $localize`:@@credentials.connected:Connected`
    : $localize`:@@credentials.signedIn:Signed in as ${login}:LOGIN:`;
}

/** The host an address the person typed names, as they would read it: `gitlab.com`. */
export function hostOf(address: string): string {
  const named = address.trim().replace(/^[a-z]+:\/\//iu, '');
  return named.split('/')[0] ?? named;
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
