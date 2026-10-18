import { TestBed } from '@angular/core/testing';

import { CHATS_LIST, PROJECTS_LIST } from '../lib/nav';
import { NavListsService } from './nav-lists.service';

describe('NavListsService', () => {
  beforeEach(() => {
    localStorage.clear();
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  it('has every list open and by recent activity at first', () => {
    const lists = TestBed.inject(NavListsService);
    expect([...lists.collapsed()]).toEqual([]);
    expect([...lists.byName()]).toEqual([]);
  });

  it('remembers in this browser which lists are closed and which are sorted by name', () => {
    const lists = TestBed.inject(NavListsService);
    lists.setExpanded('p1', false);
    lists.setExpanded(CHATS_LIST, false);
    lists.setExpanded(CHATS_LIST, true);
    lists.setSort(PROJECTS_LIST, 'name');
    lists.setSort('p1', 'name');
    lists.setSort('p1', 'recent');
    TestBed.tick();
    TestBed.resetTestingModule();
    const again = TestBed.inject(NavListsService);
    expect([...again.collapsed()]).toEqual(['p1']);
    expect([...again.byName()]).toEqual([PROJECTS_LIST]);
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
