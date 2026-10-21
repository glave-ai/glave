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

/** Between a model's company and its own name: `Anthropic · Claude Haiku 5.5`. */
const SEPARATOR = ' · ';

/** Companies by the prefix their model ids carry (`anthropic/…`), as they write their names. */
const COMPANIES: Readonly<Record<string, string>> = {
  ai21: 'AI21',
  amazon: 'Amazon',
  anthropic: 'Anthropic',
  cohere: 'Cohere',
  deepseek: 'DeepSeek',
  google: 'Google',
  'meta-llama': 'Meta',
  microsoft: 'Microsoft',
  minimax: 'MiniMax',
  mistralai: 'Mistral',
  moonshotai: 'Moonshot AI',
  nvidia: 'NVIDIA',
  openai: 'OpenAI',
  perplexity: 'Perplexity',
  qwen: 'Qwen',
  'x-ai': 'xAI',
  'z-ai': 'Z.ai',
};

/** Words in model ids that are not written in title case. */
const WORDS: Readonly<Record<string, string>> = {
  ai: 'AI',
  deepseek: 'DeepSeek',
  glm: 'GLM',
  gpt: 'GPT',
  oss: 'OSS',
  vl: 'VL',
};

/** Words a model's name joins to the number after them with a hyphen: `GPT-6`. */
const HYPHENATED: ReadonlySet<string> = new Set(['GPT']);

function wordName(word: string): string {
  return WORDS[word.toLowerCase()] ?? `${word.charAt(0).toUpperCase()}${word.slice(1)}`;
}

/**
 * A slug as people write it, in title case but for the words its makers write otherwise:
 * `claude-haiku-5.5` is `Claude Haiku 5.5`, `gpt-6-luna` is `GPT-6 Luna`, and a tag after a
 * colon goes in brackets (`llama-4:free` is `Llama 4 (Free)`).
 */
export function slugWords(slug: string): string {
  const [base = '', tag = ''] = slug.split(':');
  const words = base
    .split(/[-_\s]+/u)
    .filter((word) => word !== '')
    .map(wordName);
  const name = words
    .map((word, index) => {
      const before = words[index - 1];
      if (before === undefined) {
        return word;
      }
      return HYPHENATED.has(before) && /^\d/u.test(word) ? `-${word}` : ` ${word}`;
    })
    .join('');
  return tag === '' ? name : `${name} (${wordName(tag)})`;
}

/** A company's name from the prefix of a model id: as it writes it, else in title case. */
export function companyName(prefix: string): string {
  return COMPANIES[prefix.toLowerCase()] ?? slugWords(prefix);
}

/** The company and the model's own name a configured name gives (`Anthropic: Claude …`). */
function fromName(name: string): { readonly company: string; readonly own: string } {
  const colon = name.indexOf(': ');
  return colon === -1
    ? { company: '', own: name }
    : { company: name.slice(0, colon).trim(), own: name.slice(colon + 2).trim() };
}

/**
 * What people read for a model, `Company · Model`, never its slug: its own name from the
 * server's `name` when there is one (`Claude Haiku 5.5`, or a configured `Anthropic: Claude
 * Haiku 5.5`), else from the id's last part; its company the server's `maker`, else from that
 * name, else from the id's prefix (`anthropic/claude-haiku-5.5`). A model with no company in
 * any is its name alone.
 */
export function displayName(model: Pick<ModelInfo, 'id' | 'name' | 'maker'>): string {
  const { company, own } = names(model);
  return company === '' ? own : `${company}${SEPARATOR}${own}`;
}

/** A model's own name alone, with no company: `Claude Haiku 5.5`. */
export function shortName(model: Pick<ModelInfo, 'id' | 'name' | 'maker'>): string {
  return names(model).own;
}

function names(model: Pick<ModelInfo, 'id' | 'name' | 'maker'>): {
  readonly company: string;
  readonly own: string;
} {
  const parts = model.id.split('/').filter((part) => part !== '');
  const slug = parts.at(-1) ?? model.id;
  const prefix = parts.length > 1 ? (parts.at(-2) ?? '') : '';
  const named = fromName(model.name?.trim() ?? '');
  const own = named.own === '' ? slugWords(slug) : named.own;
  const maker = model.maker?.trim() ?? '';
  const company =
    maker !== ''
      ? maker
      : named.company !== ''
        ? named.company
        : prefix === ''
          ? ''
          : companyName(prefix);
  return { company, own };
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
      short: shortName(model),
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
