import { TestBed } from '@angular/core/testing';
import { ApiError, ErrorCode, type Filters } from '@glave/client';

import { FakeSession, provideFakeSession } from '../testing/fake-session';
import { MODELS } from '../testing/fixtures';
import { ChangesService } from './changes.service';
import { FiltersService } from './filters.service';
import { ModelsService } from './models.service';

const EUROPE = ['DE', 'FR'];

/** The service on `fake`, signed in. */
function service(fake: FakeSession): FiltersService {
  TestBed.configureTestingModule({ providers: provideFakeSession(fake) });
  return TestBed.inject(FiltersService);
}

describe('FiltersService', () => {
  it('reads the person’s filters and what Europe means, none until then', async () => {
    const fake = new FakeSession().answer('filters/get', {
      filters: { based_in: ['US'] },
      europe: EUROPE,
    });
    const filters = service(fake);
    expect(filters.filters()).toEqual({});
    await filters.load();
    expect(filters.filters()).toEqual({ based_in: ['US'] });
    expect(filters.europe()).toEqual(EUROPE);
  });

  it('shows a choice at once, keeps what the server answers and reads the models again', async () => {
    const fake = new FakeSession()
      .answer('filters/get', { filters: {}, europe: EUROPE })
      .answer('filters/set', (params) => ({
        filters: (params as { readonly filters: Filters }).filters,
        europe: EUROPE,
      }))
      .answer('model/list', { models: MODELS });
    const filters = service(fake);
    await filters.load();
    const choosing = filters.choose({ keeps_nothing: true });
    expect(filters.filters()).toEqual({ keeps_nothing: true });
    expect(await choosing).toBe(true);
    expect(fake.paramsOf('filters/set')).toEqual([{ filters: { keeps_nothing: true } }]);
    expect(TestBed.inject(ModelsService).models()).toEqual(MODELS);
    expect(filters.problem()).toBeNull();
  });

  it('puts back what the server kept, with why, when it keeps none', async () => {
    const fake = new FakeSession().answer('filters/get', { filters: {}, europe: EUROPE }).answer(
      'filters/set',
      new ApiError({
        code: ErrorCode.INVALID_PARAMS,
        message: '"usa" is not a country: name each by its two-letter code, such as US',
      }),
    );
    const filters = service(fake);
    await filters.load();
    expect(await filters.choose({ based_in: ['usa'] })).toBe(false);
    expect(filters.filters()).toEqual({});
    expect(filters.problem()).toContain('is not a country');
  });

  it('chooses nothing until it has read them, says why it could not, and reads them again', async () => {
    const fake = new FakeSession().answer(
      'filters/get',
      new ApiError({ code: ErrorCode.INTERNAL, message: 'The store is not answering.' }),
    );
    const filters = service(fake);
    await filters.load();
    expect(filters.known()).toBe(false);
    expect(filters.problem()).toContain('The store is not answering.');
    expect(await filters.choose({ keeps_nothing: true })).toBe(false);
    expect(fake.paramsOf('filters/set')).toEqual([]);
    expect(filters.filters()).toEqual({});
    fake.answer('filters/get', { filters: { based_in: ['US'] }, europe: EUROPE });
    await filters.load();
    expect(filters.known()).toBe(true);
    expect(filters.problem()).toBeNull();
    expect(filters.filters()).toEqual({ based_in: ['US'] });
  });

  it('sends quick choices one after another and shows the last', async () => {
    const waiting: ((answer: object) => void)[] = [];
    const fake = new FakeSession()
      .answer('filters/get', { filters: {}, europe: EUROPE })
      .answer('model/list', { models: MODELS })
      .answer(
        'filters/set',
        () =>
          new Promise((resolve) => {
            waiting.push(resolve);
          }),
      );
    const filters = service(fake);
    await filters.load();
    const first = filters.choose({ sort: 'price' });
    const second = filters.choose({ sort: 'speed' });
    expect(filters.filters()).toEqual({ sort: 'speed' });
    await vi.waitFor(() => {
      expect(waiting).toHaveLength(1);
    });
    waiting[0]?.({ filters: { sort: 'price' }, europe: EUROPE });
    expect(await first).toBe(true);
    expect(filters.filters()).toEqual({ sort: 'speed' });
    await vi.waitFor(() => {
      expect(waiting).toHaveLength(2);
    });
    waiting[1]?.({ filters: { sort: 'speed' }, europe: EUROPE });
    expect(await second).toBe(true);
    expect(filters.filters()).toEqual({ sort: 'speed' });
    expect(fake.paramsOf('filters/set')).toEqual([
      { filters: { sort: 'price' } },
      { filters: { sort: 'speed' } },
    ]);
  });

  it('never undoes a choice on its way with a read that answers before it', async () => {
    const fake = new FakeSession()
      .answer('filters/get', { filters: {}, europe: EUROPE })
      .answer('model/list', { models: MODELS });
    const give = fake.held('filters/set');
    const filters = service(fake);
    await filters.load();
    const choosing = filters.choose({ keeps_nothing: true });
    await vi.waitFor(() => {
      expect(fake.paramsOf('filters/set')).toHaveLength(1);
    });
    TestBed.inject(ChangesService).tell('filters');
    TestBed.tick();
    await filters.load();
    expect(fake.paramsOf('filters/get')).toHaveLength(2);
    expect(filters.filters()).toEqual({ keeps_nothing: true });
    give({ filters: { keeps_nothing: true }, europe: EUROPE });
    expect(await choosing).toBe(true);
    expect(filters.filters()).toEqual({ keeps_nothing: true });
  });
});
