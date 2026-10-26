import type { ModelInfo, RawEvent, TokenPrice } from '@glave/client';
import type { GlzMenuOption } from '@glave/glaze';

/** Between the parts of a model's line in the picker. */
const DOT = ' · ';

/** A context window, in tokens, as people in `locale` say it: `1M context`, `200K context`. */
export function contextText(tokens: number, locale: string): string {
  const size = new Intl.NumberFormat(locale, { notation: 'compact' }).format(tokens);
  return $localize`:@@models.context:${size}:TOKENS: context`;
}

/**
 * What a model's tokens cost, in and out, per million, in dollars as `locale` writes them:
 * `$0.10 in, $0.50 out per 1M tokens`.
 */
export function priceText(price: TokenPrice, locale: string): string {
  if (price.input === 0 && price.output === 0) {
    return $localize`:@@models.free:Free`;
  }
  const dollars = new Intl.NumberFormat(locale, {
    style: 'currency',
    currency: 'USD',
    minimumFractionDigits: 2,
    maximumFractionDigits: 3,
  });
  const input = dollars.format(price.input / 1_000_000);
  const output = dollars.format(price.output / 1_000_000);
  return $localize`:@@models.price:${input}:INPUT: in, ${output}:OUTPUT: out per 1M tokens`;
}

/**
 * The line under a model in the picker, for `locale`: whether it is the default, the service
 * that runs it, its context and its price, as far as the server says.
 */
export function modelHint(model: ModelInfo, locale: string): string {
  const context = model.context ?? null;
  const price = model.price ?? null;
  const parts = [
    model.default ? $localize`:@@models.default:Default` : '',
    model.providers?.[0] ?? '',
    context === null ? '' : contextText(context, locale),
    price === null ? '' : priceText(price, locale),
  ];
  return parts.filter((part) => part !== '').join(DOT);
}

/** Between a model's maker and its own name: `Anthropic · Claude Haiku 5.5`. */
const SEPARATOR = ' · ';

/**
 * What people read for a model, as the server names it: `Maker · Model`, or its name alone
 * when the server knows no maker.
 */
export function displayName(model: Pick<ModelInfo, 'name' | 'maker'>): string {
  const maker = model.maker ?? '';
  return maker === '' ? model.name : `${maker}${SEPARATOR}${model.name}`;
}

/** The thread's own choice of model: the latest `model.chosen`'s, or `null` for the default. */
export function chosenModel(events: readonly RawEvent[]): string | null {
  const chosen = events.filter((event) => event.type === 'model.chosen').at(-1);
  const model = chosen?.data['model'];
  return typeof model === 'string' && model !== '' ? model : null;
}

/**
 * The picker's options: each model by its display name, a line saying whether it is the
 * default, which service runs it, its context and its price (written for `locale`), and its own
 * name alone for the button once chosen.
 */
export function modelOptions(
  models: readonly ModelInfo[],
  locale: string,
): readonly GlzMenuOption[] {
  return models.map((model) => {
    const hint = modelHint(model, locale);
    return {
      value: model.id,
      label: displayName(model),
      short: model.name,
      ...(hint === '' ? {} : { hint }),
    };
  });
}

/** How many recent models the picker remembers. */
export const RECENT_MODELS = 5;

/** A stored list of recent models, read back: its model ids, at most `RECENT_MODELS`. */
export function toRecent(value: unknown): readonly string[] {
  return Array.isArray(value)
    ? value
        .filter((each): each is string => typeof each === 'string' && each !== '')
        .slice(0, RECENT_MODELS)
    : [];
}

/** `recent` with `model` picked just now: first, once, and at most `RECENT_MODELS` in all. */
export function withRecent(recent: readonly string[], model: string): readonly string[] {
  return [model, ...recent.filter((each) => each !== model)].slice(0, RECENT_MODELS);
}

/**
 * The picker's options in the order people reach for them: the default, then the models picked
 * lately (latest first), then everything else in the server's order.
 */
export function orderedOptions(
  options: readonly GlzMenuOption[],
  first: string | null,
  recent: readonly string[],
): readonly GlzMenuOption[] {
  const rank = (value: string): number => {
    if (value === first) {
      return -1;
    }
    const at = recent.indexOf(value);
    return at === -1 ? recent.length : at;
  };
  return options
    .map((option, index) => ({ option, index, rank: rank(option.value) }))
    .sort((one, other) => one.rank - other.rank || one.index - other.index)
    .map(({ option }) => option);
}
