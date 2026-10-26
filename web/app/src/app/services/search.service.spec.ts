import { TestBed } from '@angular/core/testing';
import { ApiError, ErrorCode, type SearchInfo } from '@glave/client';

import { FakeSession, provideFakeSession } from '../testing/fake-session';
import { ChangesService } from './changes.service';
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

  it('reads it again when the server says it changed elsewhere, as from the terminal', async () => {
    const fake = new FakeSession().answer('search/get', { search: OFF });
    TestBed.configureTestingModule({ providers: provideFakeSession(fake) });
    const search = TestBed.inject(SearchService);
    await settled();
    fake.answer('search/get', { search: ON });
    TestBed.inject(ChangesService).tell('search');
    TestBed.tick();
    await settled();
    expect(search.on()).toBe(true);
    expect(fake.paramsOf('search/get')).toHaveLength(2);
  });

  it('leaves a switch on its way to say what is so, rather than read it again meanwhile', async () => {
    let answer: (value: { search: SearchInfo }) => void = () => undefined;
    const fake = new FakeSession().answer('search/get', { search: OFF }).answer(
      'search/set',
      () =>
        new Promise<{ search: SearchInfo }>((resolve) => {
          answer = resolve;
        }),
    );
    TestBed.configureTestingModule({ providers: provideFakeSession(fake) });
    const search = TestBed.inject(SearchService);
    await search.load(true);
    const switched = search.set(true);
    await settled();
    // The server says it changed, as it does for this very switch.
    TestBed.inject(ChangesService).tell('search');
    TestBed.tick();
    expect(fake.paramsOf('search/get')).toHaveLength(2);
    answer({ search: { ...ON, usage: { calls: 0, sentence: 'No searches this month' } } });
    await switched;
    expect(search.on()).toBe(true);
    expect(search.search()).toMatchObject({ usage: { sentence: 'No searches this month' } });
    // Once it is answered, a change read again asks how often it searched, as Settings did.
    TestBed.inject(ChangesService).tell('search');
    TestBed.tick();
    expect(fake.paramsOf('search/get').at(-1)).toEqual({ usage: true });
  });

  it('reads only whether it is on as it starts, and the month’s searches when asked', async () => {
    const fake = new FakeSession().answer('search/get', { search: OFF });
    TestBed.configureTestingModule({ providers: provideFakeSession(fake) });
    const search = TestBed.inject(SearchService);
    await settled();
    await search.load(true);
    expect(fake.paramsOf('search/get')).toEqual([{ usage: false }, { usage: true }]);
  });

  it('sends each switch once the one before is answered, and the last answer stays', async () => {
    const answers: ((value: { search: SearchInfo }) => void)[] = [];
    const fake = new FakeSession().answer('search/get', { search: OFF }).answer(
      'search/set',
      () =>
        new Promise<{ search: SearchInfo }>((resolve) => {
          answers.push(resolve);
        }),
    );
    TestBed.configureTestingModule({ providers: provideFakeSession(fake) });
    const search = TestBed.inject(SearchService);
    await settled();
    const first = search.set(true);
    const second = search.set(false);
    await settled();
    // The second waits for the first's answer before it is sent.
    expect(fake.paramsOf('search/set')).toEqual([{ on: true }]);
    answers[0]?.({ search: ON });
    await first;
    await settled();
    expect(fake.paramsOf('search/set')).toEqual([{ on: true }, { on: false }]);
    expect(search.on()).toBe(false);
    answers[1]?.({ search: OFF });
    await second;
    expect(search.on()).toBe(false);
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
