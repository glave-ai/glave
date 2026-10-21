import { TestBed } from '@angular/core/testing';

import { CHATS_LIST } from '../lib/nav';
import { NavListsService } from './nav-lists.service';

describe('NavListsService', () => {
  beforeEach(() => {
    localStorage.clear();
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  it('has every list open at first', () => {
    const lists = TestBed.inject(NavListsService);
    expect([...lists.collapsed()]).toEqual([]);
  });

  it('remembers in this browser which lists are closed', () => {
    const lists = TestBed.inject(NavListsService);
    lists.setExpanded('p1', false);
    lists.setExpanded(CHATS_LIST, false);
    lists.setExpanded(CHATS_LIST, true);
    TestBed.tick();
    TestBed.resetTestingModule();
    const again = TestBed.inject(NavListsService);
    expect([...again.collapsed()]).toEqual(['p1']);
  });

  it('works without storage', () => {
    vi.spyOn(Storage.prototype, 'getItem').mockImplementation(() => {
      throw new Error('denied');
    });
    vi.spyOn(Storage.prototype, 'setItem').mockImplementation(() => {
      throw new Error('denied');
    });
    const lists = TestBed.inject(NavListsService);
    lists.setExpanded('p1', false);
    TestBed.tick();
    expect([...lists.collapsed()]).toEqual(['p1']);
  });
});
