import type { Page, Route } from '@playwright/test';
import type { EndpointInfo, Filters, ModelInfo } from '@glave/client';

import { MODELS } from './fake-models';
import { posted } from './fake-records';

/** The countries the fake server says "Europe" means. */
const EUROPE = ['DE', 'FR', 'NL'];

/** DeepSeek V4.1 Flash's providers through a router, as the fake server's catalogue says. */
const SERVING: readonly EndpointInfo[] = [
  {
    provider: 'DeepSeek',
    slug: 'deepseek',
    based_in: 'CN',
    datacenters: ['CN'],
    precision: 'eight_bit',
    keeps_nothing: false,
    price: { input: 100_000, output: 400_000 },
    speed: 90,
  },
  {
    provider: 'Groq',
    slug: 'groq',
    based_in: 'US',
    datacenters: ['US'],
    precision: 'four_bit',
    keeps_nothing: true,
    price: { input: 120_000, output: 500_000 },
    speed: 210,
  },
  {
    provider: 'Nebius',
    slug: 'nebius',
    based_in: 'NL',
    precision: 'full',
    keeps_nothing: false,
    price: { input: 200_000, output: 800_000 },
    speed: 80,
  },
];

/** The fake server's models, DeepSeek's served by a router, with what `filters` hide. */
function models(filters: Filters): readonly ModelInfo[] {
  const places = filters.based_in ?? [];
  const endpoints = SERVING.map((endpoint) => {
    const hidden = places.length > 0 && !places.includes(endpoint.based_in ?? '');
    return hidden ? { ...endpoint, hidden_by: ['based_in' as const] } : endpoint;
  });
  return MODELS.map((model) =>
    model.id === 'deepseek/deepseek-v4.1-flash' ? { ...model, endpoints } : model,
  );
}

/**
 * Answers `model/list` with a model a router serves, and keeps the person's filters among its
 * providers (`filters/get`, `filters/set`), hiding by where providers are based, as the server
 * does; every other call goes on to the fake API set up before it. What the page kept, in
 * order, is in `kept`.
 */
export async function fakeFilters(page: Page): Promise<{ readonly kept: readonly Filters[] }> {
  const kept: Filters[] = [];
  let filters: Filters = {};
  await page.route('**/*', async (route: Route) => {
    const message = await posted(route);
    if (message === null) {
      return;
    }
    let result: unknown;
    if (message.method === 'model/list') {
      result = { models: models(filters) };
    } else if (message.method === 'filters/get') {
      result = { filters, europe: EUROPE };
    } else if (message.method === 'filters/set') {
      filters = message.params['filters'] as Filters;
      kept.push(filters);
      result = { filters, europe: EUROPE };
    } else {
      await route.fallback();
      return;
    }
    await route.fulfill({
      contentType: 'application/json',
      body: JSON.stringify({ jsonrpc: '2.0', id: message.id, result }),
    });
  });
  return { kept };
}
