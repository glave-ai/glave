import { ApiError, ErrorCode, type ProviderInfo, type ProviderKind } from '@glave/client';

/** A service a person can connect a model account with, by its plain name. */
export interface Service {
  readonly kind: ProviderKind;
  readonly name: string;
}

/** The services offered, in order. */
export const SERVICES: readonly Service[] = [
  { kind: 'openai', name: 'OpenAI' },
  { kind: 'anthropic', name: 'Anthropic' },
  { kind: 'openrouter', name: 'OpenRouter' },
  { kind: 'compatible', name: 'Another server' },
];

/** What a failure the server did not explain is told as. */
const SOMETHING_WRONG = 'Something went wrong. Try again in a moment.';

/** The service of `kind`. */
export function serviceOf(kind: ProviderKind): Service {
  return SERVICES.find((service) => service.kind === kind) ?? { kind, name: kind };
}

/** Whether connecting at `kind` needs the server's address. */
export function needsAddress(kind: ProviderKind): boolean {
  return kind === 'compatible';
}

/** The link that changes an account's key: its service picked, and another server's address. */
export function changeQuery(account: ProviderInfo): Readonly<Record<string, string>> {
  const url = account.url ?? '';
  return url === '' ? { kind: account.kind } : { kind: account.kind, url };
}

/** What to tell the person when removing an account failed. */
export const REMOVE_FAILED = 'glave could not remove that account. Try again in a moment.';

/** How an account shows in a list: its service, or the address of another server. */
export function accountName(account: ProviderInfo): string {
  const url = account.url ?? '';
  return account.kind === 'compatible' && url !== '' ? url : serviceOf(account.kind).name;
}

/** The end of an account's key, to tell keys apart, or nothing for a key too short to show. */
export function keyHint(account: ProviderInfo): string {
  const hint = account.hint ?? '';
  return hint === '' ? '' : `Key ending ${hint}`;
}

/** What to tell the person when connecting failed, in a sentence they can act on. */
export function connectProblem(error: unknown): string {
  if (!(error instanceof ApiError)) {
    return SOMETHING_WRONG;
  }
  switch (error.code) {
    case ErrorCode.PROVIDER:
    case ErrorCode.INVALID_PARAMS:
    case ErrorCode.NOT_ALLOWED:
      return sentence(error.message);
    case ErrorCode.NOT_AVAILABLE:
      return 'Models are added on the computer glave runs on.';
    default:
      return SOMETHING_WRONG;
  }
}

/** `text` as a sentence: a capital first, a full stop last. */
function sentence(text: string): string {
  const trimmed = text.trim();
  if (trimmed === '') {
    return SOMETHING_WRONG;
  }
  const capital = `${trimmed.charAt(0).toUpperCase()}${trimmed.slice(1)}`;
  return /[.!?]$/u.test(capital) ? capital : `${capital}.`;
}
