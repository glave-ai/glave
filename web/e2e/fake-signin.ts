import type { ServerInfo } from '@glave/client';

import type { Answers } from './fake-records';

/** The fake server's id, which `server/discover` answers and the sessions it makes name. */
export const FAKE_SERVER = '01900000-0000-7000-8000-0000000005e7';

/**
 * What the fake glave says it is (`server/discover`), at `version`: its own, unless a test
 * says glave was updated since the page loaded.
 */
export function discovered(version = '0.0.0'): ServerInfo {
  return {
    name: 'glave',
    version,
    protocols: ['2026-10'],
    permissions: 'auto',
    modes: ['auto', 'plan', 'ask', 'full-access'],
    id: FAKE_SERVER,
  };
}

/** The code of a link that signs the app in from another computer. */
const ELSEWHERE_CODE = 'elsewhere';

/** The fragment of that link. */
export const SIGNED_IN_ELSEWHERE = `#code=${ELSEWHERE_CODE}`;

/**
 * Trading a link's code for a session (`signin/redeem`): a browser session, or for the link from
 * elsewhere a token an identity provider would give.
 */
export const SIGN_IN: Answers<null> = {
  'signin/redeem': (params) => ({
    session: params.code === ELSEWHERE_CODE ? 'signed-in-elsewhere' : 'glave_session_e2e',
    server: FAKE_SERVER,
  }),
};
