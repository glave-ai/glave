import { TestBed } from '@angular/core/testing';
import { ApiError, ErrorCode } from '@glave/client';

import { FakeSession, provideFakeSession } from '../testing/fake-session';
import { picture } from '../testing/fixtures';
import { LibraryService } from './library.service';

function setUp(fake: FakeSession): LibraryService {
  TestBed.configureTestingModule({ providers: provideFakeSession(fake) });
  return TestBed.inject(LibraryService);
}

describe('LibraryService', () => {
  it('reads the Library as the server says it, and takes an item out of it', async () => {
    const fox = picture('fox', 'p1', 't1');
    const hare = picture('hare', 'p1');
    const fake = new FakeSession()
      .answer('library/list', { items: [fox, hare] })
      .answer('library/remove', {});
    const library = setUp(fake);
    expect(library.loaded()).toBe(false);
    await library.refresh();
    expect(library.loaded()).toBe(true);
    expect(library.items()).toEqual([fox, hare]);
    await library.remove(fox);
    expect(fake.paramsOf('library/remove')).toEqual([{ project: 'p1', asset: 'fox' }]);
    expect(library.items()).toEqual([hare]);
    expect(library.error()).toBeNull();
  });

  it('says why an item could not be taken out, and keeps it', async () => {
    const gone = new ApiError({ code: ErrorCode.NO_FILE, message: 'this file is no longer here' });
    const fake = new FakeSession()
      .answer('library/list', { items: [picture('fox', 'p1')] })
      .answer('library/remove', () => gone);
    const library = setUp(fake);
    await library.refresh();
    await library.remove(picture('fox', 'p1'));
    expect(library.error()).toBe('this file is no longer here');
    expect(library.items()).toHaveLength(1);
  });
});
