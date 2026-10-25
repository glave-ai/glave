import { TestBed } from '@angular/core/testing';
import { ApiError, ErrorCode, type SearchInfo } from '@glave/client';

import { FakeSession, provideFakeSession } from '../testing/fake-session';
import { SearchService } from './search.service';

const OFF: SearchInfo = { kind: 'chosen', on: false, detail: 'Uses Exa, free.' };
const ON: SearchInfo = { kind: 'chosen', on: true, detail: 'Uses Exa, free.' };

async function settled(): Promise<void> {
  await new Promise((resolve) => setTimeout(resolve));
}

describe('SearchService', () => {
  it('reads whether the agent may search as it starts, and keeps what the server says', async () => {
    const fake = new FakeSession()
      .answer('search/get', { search: OFF })
      .answer('search/set', { search: ON });
    TestBed.configureTestingModule({ providers: provideFakeSession(fake) });
    const search = TestBed.inject(SearchService);
    await settled();
    expect(search.search()).toEqual(OFF);
    expect(search.on()).toBe(false);
    await search.set(true);
    expect(fake.paramsOf('search/set')).toEqual([{ on: true }]);
    expect(search.on()).toBe(true);
    expect(search.problem()).toBeNull();
  });

  it('keeps what it had when the server refuses, and says why', async () => {
    const fake = new FakeSession()
      .answer('search/get', { search: ON })
      .answer(
        'search/set',
        new ApiError({ code: ErrorCode.NOT_AVAILABLE, message: 'it stays on here' }),
      );
    TestBed.configureTestingModule({ providers: provideFakeSession(fake) });
    const search = TestBed.inject(SearchService);
    await settled();
    await search.set(false);
    expect(search.on()).toBe(true);
    expect(search.problem()).toBe('It stays on here.');
  });

  it('shows nothing it could not read', async () => {
    const fake = new FakeSession().answer(
      'search/get',
      new ApiError({ code: ErrorCode.NOT_ALLOWED, message: 'not yours' }),
    );
    TestBed.configureTestingModule({ providers: provideFakeSession(fake) });
    const search = TestBed.inject(SearchService);
    await settled();
    expect(search.search()).toBeNull();
    expect(search.on()).toBe(false);
  });
});
