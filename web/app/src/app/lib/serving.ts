/**
 * Who serves a model through a router, as the model picker shows it: the providers the
 * person's filters leave or hide, as the server says, their places, precision, data policy,
 * prices and speeds written for the reader, and the person's choice of who serves one model.
 */
import type {
  EndpointInfo,
  Filters,
  Hides,
  ModelInfo,
  Precision,
  Serve,
  Sort,
  TokenPrice,
} from '@glave/client';

import { plural } from './plural';

/** Between the parts of a line. */
export const DOT = ' · ';

/** Where providers may be based, as the picker offers it: a few places, or countries chosen. */
export type Place = 'anywhere' | 'us' | 'us-europe' | 'countries';

/** The countries `place` means, with "Europe" as the server says it. */
export function placeCountries(
  place: Exclude<Place, 'countries'>,
  europe: readonly string[],
): readonly string[] {
  switch (place) {
    case 'anywhere':
      return [];
    case 'us':
      return ['US'];
    case 'us-europe':
      return ['US', ...europe];
  }
}

/** Which of the picker's places `countries` are, or `countries` for any other set. */
export function placeOf(countries: readonly string[], europe: readonly string[]): Place {
  const same = (other: readonly string[]): boolean =>
    other.length === countries.length && other.every((country) => countries.includes(country));
  const places = ['anywhere', 'us', 'us-europe'] as const;
  return places.find((place) => same(placeCountries(place, europe))) ?? 'countries';
}

/** A model's providers that the person's filters leave, as the server says. */
export function shownEndpoints(model: ModelInfo): readonly EndpointInfo[] {
  return (model.endpoints ?? []).filter((endpoint) => (endpoint.hidden_by ?? []).length === 0);
}

/** A model's providers that the person's filters hide. */
export function hiddenEndpoints(model: ModelInfo): readonly EndpointInfo[] {
  return (model.endpoints ?? []).filter((endpoint) => (endpoint.hidden_by ?? []).length > 0);
}

/** Whether a router serves `model`, so who serves it is the person's to choose. */
export function routed(model: ModelInfo): boolean {
  return (model.endpoints ?? []).length > 0;
}

/** Whether the list shows `model`: one no router serves always; else while a provider is left. */
export function listed(model: ModelInfo): boolean {
  return !routed(model) || shownEndpoints(model).length > 0;
}

/** What a million tokens in and out cost together at `price`. */
function total(price: TokenPrice): number {
  return price.input + price.output;
}

/** The price of the cheapest of `endpoints` that says one, or `null`. */
export function cheapest(endpoints: readonly EndpointInfo[]): TokenPrice | null {
  const prices = endpoints.flatMap((endpoint) => (endpoint.price ? [endpoint.price] : []));
  return prices.reduce<TokenPrice | null>(
    (least, price) => (least === null || total(price) < total(least) ? price : least),
    null,
  );
}

/** The speed of the fastest of `endpoints` that says one, in tokens a second, or `null`. */
export function fastest(endpoints: readonly EndpointInfo[]): number | null {
  const speeds = endpoints.flatMap((endpoint) => (endpoint.speed ? [endpoint.speed] : []));
  return speeds.length === 0 ? null : Math.max(...speeds);
}

/**
 * `models` in the order `sort` asks: those a router serves by their cheapest or fastest
 * provider left, then the rest, each kept in the server's order where they tie.
 */
export function sortedModels(models: readonly ModelInfo[], sort: Sort): readonly ModelInfo[] {
  const key = (model: ModelInfo): number => {
    const shown = shownEndpoints(model);
    if (sort === 'speed') {
      return -(fastest(shown) ?? -Infinity);
    }
    // A model a router serves by the cheapest provider left, as its row shows it.
    const price = routed(model) ? cheapest(shown) : (model.price ?? null);
    return price === null ? Infinity : total(price);
  };
  return models
    .map((model, index) => ({ model, index, key: key(model) }))
    .sort((one, other) => one.key - other.key || one.index - other.index)
    .map(({ model }) => model);
}

/** A country, by its ISO code, as people in `locale` name it. */
export function countryName(code: string, locale: string): string {
  try {
    return new Intl.DisplayNames([locale], { type: 'region' }).of(code) ?? code;
  } catch {
    return code;
  }
}

/** How precisely a provider serves a model's weights, in a few words. */
export function precisionText(precision: Precision): string {
  switch (precision) {
    case 'four_bit':
      return $localize`:@@serving.fourBit:4-bit`;
    case 'six_bit':
      return $localize`:@@serving.sixBit:6-bit`;
    case 'eight_bit':
      return $localize`:@@serving.eightBit:8-bit`;
    case 'full':
      return $localize`:@@serving.full:full precision`;
    case 'unstated':
      return $localize`:@@serving.unstated:precision not said`;
  }
}

/** Where `endpoint`'s provider is based and runs, as far as the router says, for `locale`. */
export function whereText(endpoint: EndpointInfo, locale: string): string {
  const home = endpoint.based_in ?? null;
  if (home === null) {
    return $localize`:@@serving.homeUnsaid:Where it’s based isn’t said`;
  }
  const based = countryName(home, locale);
  const places = endpoint.datacenters ?? null;
  if (places === null) {
    return $localize`:@@serving.basedRunsUnsaid:Based in ${based}:COUNTRY:, where it runs isn’t said`;
  }
  if (places.every((place) => place === home)) {
    return $localize`:@@serving.based:Based in ${based}:COUNTRY:`;
  }
  const runs = new Intl.ListFormat(locale, { type: 'conjunction' }).format(
    places.map((place) => countryName(place, locale)),
  );
  return $localize`:@@serving.basedRuns:Based in ${based}:COUNTRY:, runs in ${runs}:PLACES:`;
}

/** What `endpoint`'s provider keeps of what it is sent. */
export function keepsText(endpoint: EndpointInfo): string {
  return endpoint.keeps_nothing
    ? $localize`:@@serving.keepsNothing:keeps nothing`
    : $localize`:@@serving.keepsSome:may keep chats for a while, never trains on them`;
}

/** The line under a provider: where it is, how precisely it serves, what it keeps. */
export function providerDetail(endpoint: EndpointInfo, locale: string): string {
  return [whereText(endpoint, locale), precisionText(endpoint.precision), keepsText(endpoint)].join(
    DOT,
  );
}

/** A price per million tokens, in and out, as `locale` writes dollars: `$0.12 in · $0.50 out`. */
export function priceLine(price: TokenPrice, locale: string): string {
  const dollars = new Intl.NumberFormat(locale, {
    style: 'currency',
    currency: 'USD',
    minimumFractionDigits: 2,
    maximumFractionDigits: 3,
  });
  const input = dollars.format(price.input / 1_000_000);
  const output = dollars.format(price.output / 1_000_000);
  return $localize`:@@serving.price:${input}:INPUT: in · ${output}:OUTPUT: out`;
}

/** A speed in tokens a second, for `locale`. */
export function speedLine(speed: number, locale: string): string {
  const count = new Intl.NumberFormat(locale).format(speed);
  return $localize`:@@serving.speed:${count}:COUNT: tokens a second`;
}

/** Why one of the person's filters hides `endpoint`, in a few words, for `locale`. */
function hiddenWhy(endpoint: EndpointInfo, by: Hides, locale: string): string {
  switch (by) {
    case 'based_in': {
      if (!endpoint.based_in) {
        return $localize`:@@serving.hiddenHome:where it’s based isn’t said`;
      }
      const country = countryName(endpoint.based_in, locale);
      return $localize`:@@serving.hiddenBased:based in ${country}:COUNTRY:`;
    }
    case 'precision':
      return precisionText(endpoint.precision);
    case 'keeps_some':
      return $localize`:@@serving.hiddenKeeps:may keep chats`;
    case 'price':
      return $localize`:@@serving.hiddenPrice:costs more`;
    case 'avoided':
      return $localize`:@@serving.hiddenAvoided:avoided`;
    case 'server':
      return $localize`:@@serving.hiddenServer:not used on this server`;
  }
}

/** The providers the filters hide, each with why: `DeepSeek (based in China)`. */
function hiddenText(endpoints: readonly EndpointInfo[], locale: string): string {
  const reasons = new Intl.ListFormat(locale, { type: 'unit' });
  const each = endpoints.map((endpoint) => {
    const provider = endpoint.provider;
    const why = reasons.format(
      (endpoint.hidden_by ?? []).map((by) => hiddenWhy(endpoint, by, locale)),
    );
    return $localize`:@@serving.hiddenOne:${provider}:PROVIDER: (${why}:WHY:)`;
  });
  return new Intl.ListFormat(locale, { type: 'conjunction' }).format(each);
}

/**
 * The line that says which providers are hidden and why: the person's filters, or this server
 * where nothing else of theirs hides them, so they aren't blamed for what isn't theirs.
 */
export function hiddenLine(endpoints: readonly EndpointInfo[], locale: string): string {
  if (endpoints.every((endpoint) => (endpoint.hidden_by ?? []).every((by) => by === 'server'))) {
    const names = new Intl.ListFormat(locale, { type: 'conjunction' }).format(
      endpoints.map((endpoint) => endpoint.provider),
    );
    return $localize`:@@serving.hiddenByServer:This server doesn’t use ${names}:PROVIDERS:.`;
  }
  const which = hiddenText(endpoints, locale);
  return $localize`:@@serving.hiddenByFilters:Your filters hide ${which}:PROVIDERS:.`;
}

/** Who serves a model, as the picker offers it: an order, or one provider by its slug. */
export type Choice = `sort:${Sort}` | `provider:${string}`;

/**
 * Who serves `model` now, as `filters` say: its own choice, else their order; none where they
 * chose neither, and the router keeps its own balance.
 */
export function choiceOf(filters: Filters, model: string): Choice | null {
  const serve = filters.serves?.[model];
  if (serve !== undefined && 'provider' in serve) {
    return `provider:${serve.provider}`;
  }
  const sort = serve?.sort ?? filters.sort ?? null;
  return sort === null ? null : `sort:${sort}`;
}

/**
 * `filters` with `choice` for `model`: none of its own where it is their order already, or
 * where `choice` is none.
 */
export function withChoice(filters: Filters, model: string, choice: Choice | null): Filters {
  const others = Object.fromEntries(
    Object.entries(filters.serves ?? {}).filter(([kept]) => kept !== model),
  );
  if (choice === null) {
    return { ...filters, serves: others };
  }
  const serve: Serve = choice.startsWith('provider:')
    ? { provider: choice.slice('provider:'.length) }
    : { sort: choice.slice('sort:'.length) as Sort };
  const ownOrder = 'sort' in serve && serve.sort === filters.sort;
  return { ...filters, serves: ownOrder ? others : { ...others, [model]: serve } };
}

/** How many providers serve a model, for `locale`: `1 provider`, `3 providers`. */
export function providersText(count: number, locale: string): string {
  return plural(
    $localize`:@@serving.providers:{${count}:COUNT:, plural, one {# provider} other {# providers}}`,
    locale,
  );
}
