import { TestBed } from '@angular/core/testing';

import { PinnedModelsService } from './pinned-models.service';

describe('PinnedModelsService', () => {
  beforeEach(() => {
    localStorage.clear();
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  it('pins nothing at first, and remembers pins in this browser', () => {
    const pins = TestBed.inject(PinnedModelsService);
    expect(pins.pinned()).toEqual([]);
    pins.pinned.set(['openai/gpt-6-luna', 'hope-12b']);
    TestBed.tick();
    TestBed.resetTestingModule();
    expect(TestBed.inject(PinnedModelsService).pinned()).toEqual(['openai/gpt-6-luna', 'hope-12b']);
  });

  it('works without storage', () => {
    vi.spyOn(Storage.prototype, 'getItem').mockImplementation(() => {
      throw new Error('denied');
    });
    vi.spyOn(Storage.prototype, 'setItem').mockImplementation(() => {
      throw new Error('denied');
    });
    const pins = TestBed.inject(PinnedModelsService);
    pins.pinned.set(['a']);
    TestBed.tick();
    expect(pins.pinned()).toEqual(['a']);
  });
});
