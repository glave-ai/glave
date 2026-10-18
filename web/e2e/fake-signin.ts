/** The fake server's id, which `server/discover` answers and the sessions it makes name. */
export const FAKE_SERVER = '01900000-0000-7000-8000-0000000005e7';

/** The code of a link that signs the app in from another computer. */
const ELSEWHERE_CODE = 'elsewhere';

/** The fragment of that link. */
export const SIGNED_IN_ELSEWHERE = `#code=${ELSEWHERE_CODE}`;

/**
 * The answer to trading a link's code for a session (`signin/redeem`): a browser session, or
 * for the link from elsewhere a token an identity provider would give; else `undefined`.
 */
export function signIn(method: string, params: Record<string, unknown>): unknown {
  if (method !== 'signin/redeem') {
    return undefined;
  }
  const elsewhere = params['code'] === ELSEWHERE_CODE;
  return {
    session: elsewhere ? 'signed-in-elsewhere' : 'glave_session_e2e',
    server: FAKE_SERVER,
  };
}
