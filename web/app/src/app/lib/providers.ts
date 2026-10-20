import { ApiError, ErrorCode, type ProviderInfo, type ProviderKind } from '@glave/client';

import { SOMETHING_WRONG, sentence } from './text';

/** A service a person can connect a model account with, by its plain name. */
export interface Service {
  readonly kind: ProviderKind;
  readonly name: string;
  /** What it is, in a line under its name. */
  readonly hint: string;
}

/**
 * The services offered, in order: OpenRouter first, one key for every major model, then any
 * service that speaks OpenAI's API, which is how glave talks with models.
 */
export const SERVICES: readonly Service[] = [
  {
    kind: 'openrouter',
    name: 'OpenRouter',
    hint: 'One key for every major model, each priced as you use it.',
  },
  { kind: 'openai', name: 'OpenAI', hint: 'GPT models, with a key from OpenAI.' },
  {
    kind: 'compatible',
    name: 'Another server',
    hint: 'Any server that speaks OpenAI’s API, such as Ollama, LM Studio or vLLM.',
  },
];

/** A service glave still keeps accounts with, but no longer offers to add. */
const ANTHROPIC: Service = {
  kind: 'anthropic',
  name: 'Anthropic',
  hint: 'Claude models, with a key from Anthropic.',
};

/** Every service glave keeps accounts with. */
const KNOWN: readonly Service[] = [...SERVICES, ANTHROPIC];

/** Whether `value` names a service glave keeps accounts with. */
export function isServiceKind(value: unknown): value is ProviderKind {
  return KNOWN.some((service) => service.kind === value);
}

/** The service of `kind`. */
export function serviceOf(kind: ProviderKind): Service {
  return KNOWN.find((service) => service.kind === kind) ?? { kind, name: kind, hint: '' };
}

/** The services to choose from: those offered, and `kind` too when a link picks another. */
export function servicesFor(kind: ProviderKind): readonly Service[] {
  return SERVICES.some((service) => service.kind === kind)
    ? SERVICES
    : [...SERVICES, serviceOf(kind)];
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

/** The end of an account's key, to tell keys apart, and how many models it offers. */
export function keyHint(account: ProviderInfo): string {
  const hint = account.hint ?? '';
  const count = account.models.length;
  const models = count === 1 ? '1 model' : `${String(count)} models`;
  return hint === '' ? models : `Key ending ${hint} · ${models}`;
}

/** What to tell the person when refreshing the models failed. */
export function refreshProblem(error: unknown): string {
  return error instanceof ApiError && error.code === ErrorCode.PROVIDER
    ? sentence(error.message)
    : SOMETHING_WRONG;
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
