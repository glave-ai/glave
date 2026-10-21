import { ApiError, ErrorCode, type ProviderInfo, type ProviderKind } from '@glave/client';

import { plural } from './plural';
import { sentence, somethingWrong } from './text';

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
const OFFERED: readonly ProviderKind[] = ['openrouter', 'openai', 'compatible'];

/** Every service glave keeps accounts with: those offered, and one no longer offered. */
const KNOWN: readonly ProviderKind[] = [...OFFERED, 'anthropic'];

/** Whether `value` names a service glave keeps accounts with. */
export function isServiceKind(value: unknown): value is ProviderKind {
  return KNOWN.some((kind) => kind === value);
}

/** The service of `kind`, by its plain name, with what it is. */
export function serviceOf(kind: ProviderKind): Service {
  switch (kind) {
    case 'openrouter':
      return {
        kind,
        name: 'OpenRouter',
        hint: $localize`:@@providers.openrouter:One key for every major model, each priced as you use it.`,
      };
    case 'openai':
      return {
        kind,
        name: 'OpenAI',
        hint: $localize`:@@providers.openai:GPT models, with a key from OpenAI.`,
      };
    case 'compatible':
      return {
        kind,
        name: $localize`:@@providers.compatible.name:Another server`,
        hint: $localize`:@@providers.compatible:Any server that speaks OpenAI’s API, such as Ollama, LM Studio or vLLM.`,
      };
    case 'anthropic':
      return {
        kind,
        name: 'Anthropic',
        hint: $localize`:@@providers.anthropic:Claude models, with a key from Anthropic.`,
      };
  }
}

/** The services offered, in order. */
export function offeredServices(): readonly Service[] {
  return OFFERED.map(serviceOf);
}

/** The services to choose from: those offered, and `kind` too when a link picks another. */
export function servicesFor(kind: ProviderKind): readonly Service[] {
  const offered = OFFERED.includes(kind) ? OFFERED : [...OFFERED, kind];
  return offered.map(serviceOf);
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

/** How an account shows in a list: its service, or the address of another server. */
export function accountName(account: ProviderInfo): string {
  const url = account.url ?? '';
  return account.kind === 'compatible' && url !== '' ? url : serviceOf(account.kind).name;
}

/**
 * The end of an account's key, to tell keys apart, and how many models it offers, written for
 * `locale`.
 */
export function keyHint(account: ProviderInfo, locale: string): string {
  const hint = account.hint ?? '';
  const count = account.models.length;
  return plural(
    hint === ''
      ? $localize`:@@providers.models:{${count}:COUNT:, plural, one {# model} other {# models}}`
      : $localize`:@@providers.keyModels:Key ending ${hint}:HINT: · {${count}:COUNT:, plural, one {# model} other {# models}}`,
    locale,
  );
}

/** What to tell the person when refreshing the models failed. */
export function refreshProblem(error: unknown): string {
  return error instanceof ApiError && error.code === ErrorCode.PROVIDER
    ? sentence(error.message)
    : somethingWrong();
}

/** What to tell the person when connecting failed, in a sentence they can act on. */
export function connectProblem(error: unknown): string {
  if (!(error instanceof ApiError)) {
    return somethingWrong();
  }
  switch (error.code) {
    case ErrorCode.PROVIDER:
    case ErrorCode.INVALID_PARAMS:
    case ErrorCode.NOT_ALLOWED:
      return sentence(error.message);
    case ErrorCode.NOT_AVAILABLE:
      return $localize`:@@providers.notHere:Models are added on the computer glave runs on.`;
    default:
      return somethingWrong();
  }
}
