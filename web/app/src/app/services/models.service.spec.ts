import { TestBed } from '@angular/core/testing';
import { ApiError, ErrorCode } from '@glave/client';

import { FakeSession, provideFakeSession } from '../testing/fake-session';
import { MODELS } from '../testing/fixtures';
import { ModelsService } from './models.service';

describe('ModelsService', () => {
  it('reads the models once, and names them', async () => {
    const fake = new FakeSession().answer('model/list', { models: MODELS });
    TestBed.configureTestingModule({ providers: provideFakeSession(fake) });
    const models = TestBed.inject(ModelsService);
    expect(models.default()).toBeNull();
    expect(models.choosable()).toBe(false);
    await Promise.all([models.load(), models.load()]);
    expect(fake.paramsOf('model/list')).toHaveLength(1);
    expect(models.default()).toBe('gpt-6-luna');
    expect(models.choosable()).toBe(true);
  });

  it('reads again after a failure', async () => {
    const fake = new FakeSession().answer(
      'model/list',
      new ApiError({ code: ErrorCode.INTERNAL, message: 'down' }),
    );
    TestBed.configureTestingModule({ providers: provideFakeSession(fake) });
    const models = TestBed.inject(ModelsService);
    await models.load();
    expect(models.models()).toEqual([]);
    fake.answer('model/list', { models: MODELS });
    await models.load();
    expect(models.models()).toHaveLength(2);
  });

  it('takes the first model as the default when the server marks none', async () => {
    const fake = new FakeSession().answer('model/list', {
      models: [{ id: 'only', default: false }],
    });
    TestBed.configureTestingModule({ providers: provideFakeSession(fake) });
    const models = TestBed.inject(ModelsService);
    await models.load();
    expect(models.default()).toBe('only');
  });
});
