import { ApiError, ErrorCode, type ProviderInfo } from '@glave/client';

import {
  offeredServices,
  accountName,
  changeQuery,
  connectProblem,
  keyHint,
  needsAddress,
  refreshProblem,
  serviceOf,
  servicesFor,
} from './providers';

const OPENAI: ProviderInfo = {
  id: '01900000-0000-7000-8000-000000000301',
  kind: 'openai',
  models: ['gpt-6'],
};

describe('providers', () => {
  it('offers OpenRouter first and another server last, by plain names', () => {
    expect(offeredServices().map((service) => service.name)).toEqual([
      'OpenRouter',
      'OpenAI',
      'Another server',
    ]);
    expect(serviceOf('openrouter').name).toBe('OpenRouter');
    expect(serviceOf('anthropic').name).toBe('Anthropic');
    expect(servicesFor('openai')).toEqual(offeredServices());
    expect(servicesFor('anthropic').map((service) => service.kind)).toContain('anthropic');
    expect(needsAddress('compatible')).toBe(true);
    expect(needsAddress('anthropic')).toBe(false);
  });

  it('names an account by its service, or another server by its address', () => {
    expect(accountName(OPENAI)).toBe('OpenAI');
    expect(keyHint(OPENAI, 'en-US')).toBe('1 model');
    const local = { ...OPENAI, kind: 'compatible', url: 'http://localhost:11434/v1' } as const;
    expect(accountName(local)).toBe('http://localhost:11434/v1');
    expect(keyHint({ ...OPENAI, models: ['a', 'b'] }, 'en-US')).toBe('2 models');
    expect(changeQuery(OPENAI)).toEqual({ kind: 'openai' });
    expect(changeQuery(local)).toEqual({ kind: 'compatible', url: 'http://localhost:11434/v1' });
  });

  it("says what went wrong in the server's words when it said what to do", () => {
    const refused = new ApiError({
      code: ErrorCode.PROVIDER,
      message: 'OpenAI did not accept that key: check that you copied all of it',
      data: { failure: 'refused' },
    });
    expect(connectProblem(refused)).toBe(
      'OpenAI did not accept that key: check that you copied all of it.',
    );
    const elsewhere = new ApiError({ code: ErrorCode.NOT_AVAILABLE, message: 'not here' });
    expect(connectProblem(elsewhere)).toBe('Models are added on the computer glave runs on.');
    const down = new ApiError({ code: ErrorCode.INTERNAL, message: 'database is locked' });
    expect(connectProblem(down)).toBe('Something went wrong. Try again in a moment.');
    expect(connectProblem(new Error('network'))).toBe(
      'Something went wrong. Try again in a moment.',
    );
  });

  it('says why the models could not be refreshed in the server’s words', () => {
    const busy = new ApiError({
      code: ErrorCode.PROVIDER,
      message: 'OpenRouter is busy right now: try again in a minute',
      data: { failure: 'unavailable' },
    });
    expect(refreshProblem(busy)).toBe('OpenRouter is busy right now: try again in a minute.');
    expect(refreshProblem(new Error('network'))).toBe(
      'Something went wrong. Try again in a moment.',
    );
  });
});
