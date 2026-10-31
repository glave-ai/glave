import type { EndpointInfo, ModelInfo } from '@glave/client';

import {
  cheapest,
  choiceOf,
  fastest,
  hiddenText,
  listed,
  placeCountries,
  placeOf,
  priceLine,
  providerDetail,
  providersText,
  shownEndpoints,
  sortedModels,
  withChoice,
} from './serving';

const EUROPE = ['DE', 'FR', 'NL'];

/** One of `slug`'s endpoints, based in `home`, as the server says it. */
function endpoint(
  slug: string,
  home: string | null,
  more: Partial<EndpointInfo> = {},
): EndpointInfo {
  return {
    provider: slug.toUpperCase(),
    slug,
    based_in: home,
    precision: 'eight_bit',
    keeps_nothing: false,
    price: { input: 100_000, output: 400_000 },
    speed: 90,
    ...more,
  };
}

const FLASH: ModelInfo = {
  id: 'deepseek/deepseek-v4.1-flash',
  name: 'DeepSeek V4.1 Flash',
  maker: 'DeepSeek',
  default: false,
  endpoints: [
    endpoint('deepseek', 'CN', { hidden_by: ['based_in'] }),
    endpoint('fireworks', 'US', { price: { input: 150_000, output: 600_000 }, speed: 130 }),
    endpoint('groq', 'US', { price: { input: 120_000, output: 500_000 }, speed: 210 }),
  ],
};

const DIRECT: ModelInfo = { id: 'claude-haiku-5-5', name: 'Claude Haiku 5.5', default: true };

describe('serving', () => {
  it('names the places the picker offers, and finds them again from their countries', () => {
    expect(placeCountries('anywhere', EUROPE)).toEqual([]);
    expect(placeCountries('us-europe', EUROPE)).toEqual(['US', 'DE', 'FR', 'NL']);
    expect(placeOf([], EUROPE)).toBe('anywhere');
    expect(placeOf(['US'], EUROPE)).toBe('us');
    expect(placeOf(['NL', 'US', 'FR', 'DE'], EUROPE)).toBe('us-europe');
    expect(placeOf(['US', 'CA'], EUROPE)).toBe('countries');
  });

  it('keeps the providers the server says the filters leave, and their cheapest and fastest', () => {
    const shown = shownEndpoints(FLASH);
    expect(shown.map((each) => each.slug)).toEqual(['fireworks', 'groq']);
    expect(cheapest(shown)).toEqual({ input: 120_000, output: 500_000 });
    expect(fastest(shown)).toBe(210);
    expect(cheapest([])).toBeNull();
    expect(fastest([endpoint('slow', 'US', { speed: null })])).toBeNull();
  });

  it('lists a model no router serves always, and one a router serves while a provider is left', () => {
    expect(listed(DIRECT)).toBe(true);
    expect(listed(FLASH)).toBe(true);
    const hidden = {
      ...FLASH,
      endpoints: [endpoint('deepseek', 'CN', { hidden_by: ['based_in'] })],
    };
    expect(listed(hidden)).toBe(false);
  });

  it('orders models by their cheapest or fastest provider left, those with neither last', () => {
    const dear: ModelInfo = {
      ...FLASH,
      id: 'dear',
      endpoints: [endpoint('a', 'US', { price: { input: 900_000, output: 900_000 }, speed: 300 })],
    };
    expect(sortedModels([dear, DIRECT, FLASH], 'price').map((each) => each.id)).toEqual([
      FLASH.id,
      'dear',
      DIRECT.id,
    ]);
    expect(sortedModels([FLASH, DIRECT, dear], 'speed').map((each) => each.id)).toEqual([
      'dear',
      FLASH.id,
      DIRECT.id,
    ]);
  });

  it('says where a provider is, how precisely it serves and what it keeps, never guessing', () => {
    expect(
      providerDetail(endpoint('groq', 'US', { datacenters: ['US'], keeps_nothing: true }), 'en'),
    ).toBe('Based in United States · 8-bit · keeps nothing');
    expect(providerDetail(endpoint('fireworks', 'US'), 'en')).toBe(
      'Based in United States, where it runs isn’t said · 8-bit · may keep chats for a while, never trains on them',
    );
    expect(
      providerDetail(
        endpoint('alibaba', 'CN', { datacenters: ['SG', 'CN'], precision: 'full' }),
        'en',
      ),
    ).toBe(
      'Based in China, runs in Singapore and China · full precision · may keep chats for a while, never trains on them',
    );
    expect(providerDetail(endpoint('unsaid', null, { precision: 'unstated' }), 'en')).toBe(
      'Where it’s based isn’t said · precision not said · may keep chats for a while, never trains on them',
    );
  });

  it('writes prices, counts and what the filters hide for the reader', () => {
    expect(priceLine({ input: 120_000, output: 500_000 }, 'en')).toBe('$0.12 in · $0.50 out');
    expect(providersText(1, 'en')).toBe('1 provider');
    expect(providersText(3, 'en')).toBe('3 providers');
    const hidden = [
      endpoint('deepseek', 'CN', { hidden_by: ['based_in'] }),
      endpoint('slow', 'US', { precision: 'four_bit', hidden_by: ['precision', 'keeps_some'] }),
    ];
    expect(hiddenText(hidden, 'en')).toBe(
      'DEEPSEEK (based in China) and SLOW (4-bit, may keep chats)',
    );
    const ruledOut = [endpoint('deepseek', 'CN', { hidden_by: ['based_in', 'server'] })];
    expect(hiddenText(ruledOut, 'en')).toBe(
      'DEEPSEEK (based in China, not used on this server)',
    );
  });

  it('keeps who serves a model with the filters, none of its own where it is their order', () => {
    expect(choiceOf({}, FLASH.id), 'neither chosen: the router’s own balance').toBeNull();
    expect(choiceOf({ sort: 'speed' }, FLASH.id)).toBe('sort:speed');
    expect(withChoice({}, FLASH.id, 'sort:price').serves, 'lowest price, asked for').toEqual({
      [FLASH.id]: { sort: 'price' },
    });
    const chosen = withChoice({ sort: 'price' }, FLASH.id, 'provider:groq');
    expect(chosen.serves).toEqual({ [FLASH.id]: { provider: 'groq' } });
    expect(choiceOf(chosen, FLASH.id)).toBe('provider:groq');
    expect(withChoice(chosen, FLASH.id, 'sort:speed').serves).toEqual({
      [FLASH.id]: { sort: 'speed' },
    });
    expect(withChoice(chosen, FLASH.id, 'sort:price').serves, 'their order already').toEqual({});
    expect(withChoice(chosen, FLASH.id, null).serves).toEqual({});
  });
});
