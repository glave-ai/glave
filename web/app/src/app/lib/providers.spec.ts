import { ApiError, ErrorCode, type ProviderInfo } from '@glave/client';

import {
  SERVICES,
  accountName,
  changeQuery,
  connectProblem,
  keyHint,
  needsAddress,
  serviceOf,
} from './providers';

const OPENAI: ProviderInfo = {
  id: '01900000-0000-7000-8000-000000000301',
  kind: 'openai',
  hint: 'a1b2',
  models: ['gpt-6'],
};

describe('providers', () => {
  it('offers the services by plain names, another server last', () => {
    expect(SERVICES.map((service) => service.name)).toEqual([
      'OpenAI',
      'Anthropic',
      'OpenRouter',
      'Another server',
    ]);
    expect(serviceOf('openrouter').name).toBe('OpenRouter');
    expect(needsAddress('compatible')).toBe(true);
    expect(needsAddress('anthropic')).toBe(false);
  });

  it('names an account by its service, or another server by its address', () => {
    expect(accountName(OPENAI)).toBe('OpenAI');
    expect(keyHint(OPENAI)).toBe('Key ending a1b2');
    const local = { ...OPENAI, kind: 'compatible', url: 'http://localhost:11434/v1' } as const;
    expect(accountName(local)).toBe('http://localhost:11434/v1');
    expect(keyHint({ ...OPENAI, hint: undefined })).toBe('');
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
});
