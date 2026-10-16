import { TestBed } from '@angular/core/testing';

import { RecentModelsService } from './recent-models.service';

describe('RecentModelsService', () => {
  beforeEach(() => {
    localStorage.clear();
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  it('knows none at first, and remembers picks in this browser, the latest first', () => {
    const recent = TestBed.inject(RecentModelsService);
    expect(recent.recent()).toEqual([]);
    recent.use('openai/gpt-6-luna');
    recent.use('hope-12b');
    TestBed.tick();
    TestBed.resetTestingModule();
    expect(TestBed.inject(RecentModelsService).recent()).toEqual(['hope-12b', 'openai/gpt-6-luna']);
  });

  it('works without storage', () => {
    vi.spyOn(Storage.prototype, 'getItem').mockImplementation(() => {
      throw new Error('denied');
    });
    vi.spyOn(Storage.prototype, 'setItem').mockImplementation(() => {
      throw new Error('denied');
    });
    const recent = TestBed.inject(RecentModelsService);
    recent.use('a');
    TestBed.tick();
    expect(recent.recent()).toEqual(['a']);
  });
});
