import { ApiError, type ConnectionInfo, ErrorCode } from '@glave/client';

import { sentence, somethingWrong, unreached } from './text';

/** Failures that are glave's own, whose words are not for the person. */
const OWN: readonly number[] = [ErrorCode.INTERNAL, ErrorCode.PARSE, ErrorCode.INVALID_REQUEST];

/** Where a service is: its address's host, or the whole address when it has none. */
export function serviceHost(connection: ConnectionInfo): string {
  try {
    return new URL(connection.url).host;
  } catch {
    return connection.url;
  }
}

/** What the person calls a service: its name in their settings, else where it is. */
export function serviceName(connection: ConnectionInfo): string {
  const name = connection.name?.trim() ?? '';
  return name === '' ? serviceHost(connection) : name;
}

/**
 * What to tell the person when a call about a connection failed: the server's own sentence for
 * a refusal, the plain failure for one of glave's own, and that glave could not be reached when
 * nothing answered.
 */
export function connectionProblem(error: unknown): string {
  if (!(error instanceof ApiError)) {
    return unreached();
  }
  return OWN.includes(error.code) ? somethingWrong() : sentence(error.message);
}
