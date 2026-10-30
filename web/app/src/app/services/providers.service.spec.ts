import { TestBed } from '@angular/core/testing';
import { ApiError, ErrorCode, type ProviderInfo } from '@glave/client';

import { FakeSession, provideFakeSession } from '../testing/fake-session';
import { MODELS } from '../testing/fixtures';
import { ModelsService } from './models.service';
import { ProvidersService } from './providers.service';

const ACCOUNT: ProviderInfo = {
  id: '01900000-0000-7000-8000-000000000301',
  kind: 'openai',
  models: ['gpt-6-luna'],
};

describe('ProvidersService', () => {
  it('connects an account, then reads the accounts and the models again', async () => {
    let connected = false;
    const fake = new FakeSession()
      .answer('provider/connect', () => {
        connected = true;
        return { provider: ACCOUNT };
      })
      .answer('provider/list', () => ({ providers: connected ? [ACCOUNT] : [] }))
      .answer('model/list', () => ({ models: connected ? MODELS : [] }));
    TestBed.configureTestingModule({ providers: provideFakeSession(fake) });
    const providers = TestBed.inject(ProvidersService);
    const models = TestBed.inject(ModelsService);
    expect(await models.none()).toBe(true);
    await providers.connect({ kind: 'openai', key: 'test-key' });
    expect(fake.paramsOf('provider/connect')).toEqual([{ kind: 'openai', key: 'test-key' }]);
    expect(providers.accounts()).toEqual([ACCOUNT]);
    expect(await models.none()).toBe(false);
    expect(models.default()).toBe('gpt-6-luna');
  });

  it('passes a refusal on, keeping nothing', async () => {
    const refused = new ApiError({ code: ErrorCode.PROVIDER, message: 'no' });
    const fake = new FakeSession()
      .answer('provider/connect', refused)
      .answer('provider/list', { providers: [] });
    TestBed.configureTestingModule({ providers: provideFakeSession(fake) });
    const providers = TestBed.inject(ProvidersService);
    await expect(providers.connect({ kind: 'openai', key: 'bad' })).rejects.toBe(refused);
    expect(providers.accounts()).toEqual([]);
  });

  it('removes an account by its id', async () => {
    const fake = new FakeSession()
      .answer('provider/list', { providers: [] })
      .answer('model/list', { models: [] });
    TestBed.configureTestingModule({ providers: provideFakeSession(fake) });
    await TestBed.inject(ProvidersService).remove(ACCOUNT.id);
    expect(fake.paramsOf('provider/remove')).toEqual([{ provider: ACCOUNT.id }]);
  });
});
